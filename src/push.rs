use std::io::{self};

use std::sync::atomic::AtomicUsize;
use std::{
    collections::HashSet,
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use futures::StreamExt;

use oci_distribution::secrets::RegistryAuth;
use once_cell::sync::Lazy;
use tempfile::tempdir_in;

const NIX_DB_DIR: &str = "/nix/var/nix/db";
static NIX_DB_FILE: Lazy<String> = Lazy::new(|| format!("{}/db.sqlite", NIX_DB_DIR));
const NAR_CONTENT_TYPE: &str = "application/x-nix-nar";
const NARINFO_CONTENT_TYPE: &str = "text/x-nix-narinfo";

use crate::nix::sign::{NixKeyPair, NixSignatureList};
use crate::nix::{NarInfo, NixHash};
use crate::registry::{OciItem, OciLocation, RegistryOptions};
use crate::{
    error::Error,
    nix,
    options::{InitializeOptions, PushOptions, PushSubcommands},
    registry,
};

fn nix_db_connection(options: &PushOptions) -> Result<rusqlite::Connection, Error> {
    match tempdir_in(NIX_DB_DIR) {
        Ok(probe_dir) => {
            // NIX_DB_DIR is writable
            probe_dir.close()?;
            let db_uri = format!("file:{}?mode=ro", *NIX_DB_FILE);
            Ok(rusqlite::Connection::open(db_uri)?)
        }
        Err(_) => {
            if options.allow_immutable_db {
                log::warn!("open nix store database in immutable mode");
                let db_uri = format!("file:{}?immutable=1", *NIX_DB_FILE);
                Ok(rusqlite::Connection::open(db_uri)?)
            } else {
                Err(Error::NixDbFolderNotWritable(NIX_DB_DIR.to_string()))
            }
        }
    }
}

fn get_auth() -> RegistryAuth {
    let username = match env::var("ORANC_USERNAME") {
        Ok(u) => u,
        Err(e) => {
            log::info!(
                "use anonymous auth, invalid environment variable `ORANC_USERNAME`: {}",
                e
            );
            return RegistryAuth::Anonymous;
        }
    };
    let password = match env::var("ORANC_PASSWORD") {
        Ok(u) => u,
        Err(e) => {
            log::info!(
                "use anonymous auth, invalid environment variable `ORANC_PASSWORD`: {}",
                e
            );
            return RegistryAuth::Anonymous;
        }
    };
    RegistryAuth::Basic(username, password)
}

fn get_nix_key_pair() -> Result<NixKeyPair, Error> {
    let sk_str = env::var("ORANC_SIGNING_KEY").map_err(Error::InvalidSigningKeyEnv)?;
    NixKeyPair::from_secret_key_str(&sk_str)
}

fn build_nix_cache_info(options: &PushOptions, initialize_options: &InitializeOptions) -> String {
    format!(
        "StoreDir: {store_dir}
WantMassQuery: {mass_query}
Priority: {priority}
",
        store_dir = options.store_dir,
        mass_query = if initialize_options.no_mass_query {
            0
        } else {
            1
        },
        priority = initialize_options.priority
    )
}

async fn push_one(
    options: &PushOptions,
    auth: &RegistryAuth,
    key_pair: &NixKeyPair,
    id: i64,
    failed: &AtomicBool,
    task_counter: &AtomicUsize,
    total_tasks: usize,
) -> Result<(), Error> {
    let not_failed = || {
        if failed.load(Ordering::Relaxed) {
            Err(Error::EarlyStop)
        } else {
            Ok(())
        }
    };
    let task_num = task_counter.fetch_add(1, Ordering::Relaxed);
    let task_header = format!(
        "{task_num:>width$}/{total_tasks}",
        width = total_tasks.to_string().len()
    );

    not_failed()?;

    let to_put = tokio::task::spawn_blocking({
        let options = options.clone();
        let key_pair = key_pair.clone();
        move || {
            // this function runs in parallel and use its own connections
            let conn = nix_db_connection(&options)?;

            let path_info = nix::query_path_info(&conn, id)?;
            let store_path = path_info.path.clone();
            let store_path_hash = nix::store_path_to_hash(&options, &store_path)?;

            log::info!("[{task_header}] pushing  '{}'...", store_path);

            let nar_info_filename = format!("{store_path_hash}.narinfo");
            let mut nar_data = vec![];
            let mut nar_encoder = nix_nar::Encoder::new(&store_path);
            io::copy(&mut nar_encoder, &mut nar_data)?;
            let nar_size = nar_data.len();
            let nar_hash = NixHash::hash_data(&nar_data[..]);
            let mut nar_file_data = vec![];
            zstd::stream::copy_encode(&nar_data[..], &mut nar_file_data, options.zstd_level)?;
            drop(nar_data); // save some memory
            let nar_file_size = nar_file_data.len();
            let nar_file_hash = NixHash::hash_data(&nar_file_data[..]);

            let expected_nar_size: usize = path_info
                .nar_size
                .try_into()
                .map_err(Error::InvalidNarSize)?;
            if nar_size != expected_nar_size {
                return Err(Error::NarSizeNotMatch(path_info.nar_size, nar_size));
            }

            let nar_filename = format!("{}.nar.zst", nar_hash.base32);
            let nar_file_url = format!("nar/{nar_filename}");
            let references: Vec<String> = path_info
                .reference_store_paths
                .iter()
                .map(|p| nix::strip_store_dir(&options, p))
                .collect::<Result<_, _>>()?;
            let deriver: Option<String> = path_info
                .deriver_store_paths
                .map_or(Ok::<_, Error>(None), |p| {
                    Ok(Some(nix::strip_store_dir(&options, &p)?))
                })?;
            let nar_info_fingerprint = nix::nar_info_fingerprint(
                &options.store_dir,
                &store_path,
                &nar_hash,
                nar_size,
                &references,
            );
            let nar_info_sign = key_pair.sign(nar_info_fingerprint.as_bytes())?;
            let mut sig_list = NixSignatureList::from_optional_str(&path_info.sigs)?;
            sig_list.merge(&key_pair, nar_info_fingerprint.as_bytes(), nar_info_sign)?;
            let nar_info = NarInfo {
                store_path,
                url: nar_file_url.clone(),
                compression: "zstd".to_owned(),
                file_hash: nar_file_hash,
                file_size: nar_file_size,
                nar_hash,
                nar_size,
                references,
                deriver,
                sigs: sig_list,
                ca: path_info.ca,
            };
            let nar_info_content = nar_info.to_string();
            log::debug!("[{task_header}] narinfo:\n{nar_info_content}");
            let nar_info_data = nar_info_content.into_bytes();

            Ok(vec![
                // push nar first
                (
                    OciLocation {
                        registry: options.registry.clone(),
                        repository: options.repository.clone(),
                        key: nar_file_url,
                    },
                    OciItem {
                        content_type: Some(NAR_CONTENT_TYPE.to_owned()),
                        data: nar_file_data,
                    },
                ),
                (
                    OciLocation {
                        registry: options.registry,
                        repository: options.repository,
                        key: nar_info_filename,
                    },
                    OciItem {
                        content_type: Some(NARINFO_CONTENT_TYPE.to_owned()),
                        data: nar_info_data,
                    },
                ),
            ])
        }
    })
    .await??;

    let mut ctx = RegistryOptions::from_push_options(options).context(auth.clone());
    for (location, item) in to_put {
        registry::put(&mut ctx, &location, item).await?;
    }
    Ok(())
}

async fn handle_push_result(r: Result<(), Error>, failed: &AtomicBool) {
    if let Err(e) = r {
        match e {
            Error::EarlyStop => {
                // do nothing
            }
            e => {
                log::error!("{}", e); // auto locked
                failed.store(true, Ordering::Relaxed);
            }
        }
    }
}

pub async fn push_main(options: PushOptions) -> Result<(), Error> {
    let auth = get_auth();
    if let Some(cmd) = &options.subcommand {
        match cmd {
            PushSubcommands::Initialize(initialize_options) => {
                push_initialize_main(auth, options.clone(), initialize_options.clone()).await?
            }
        }
        return Ok(());
    }
    // real main for push
    push(&auth, &options).await?;
    Ok(())
}

pub async fn push(auth: &RegistryAuth, options: &PushOptions) -> Result<(), Error> {
    let conn = nix_db_connection(options)?;

    let lines = std::io::stdin().lines();
    let mut input_paths = HashSet::new();
    for result in lines {
        let line = result?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let caonoicalized = nix::canonicalize_store_path_input(&options.store_dir, trimmed)?;
            input_paths.insert(caonoicalized);
        }
    }
    log::debug!("input paths size: {}", input_paths.len());
    log::trace!("input paths: {:#?}", input_paths);
    let id_closures = if options.no_closure {
        nix::query_db_ids(&conn, input_paths)?
    } else {
        let ids = nix::query_db_ids(&conn, input_paths)?;
        nix::compute_closure(&conn, ids)?
    };
    log::debug!("id closure size: {}", id_closures.len());
    log::trace!("id closure: {:#?}", id_closures);
    let key_pair = get_nix_key_pair()?;
    let mut filtered = HashSet::new();
    for id in id_closures {
        if nix::filter_id(options, &key_pair, &conn, id)? {
            filtered.insert(id);
        }
    }
    let task_counter = AtomicUsize::new(1);
    let total_tasks = filtered.len();
    let failed = AtomicBool::new(false);
    log::debug!("filtered size: {}", filtered.len());
    log::trace!("filtered: {:#?}", filtered);
    log::info!("start {total_tasks} tasks...");
    let pushes = futures::stream::iter(filtered.into_iter().map(|id| {
        push_one(
            options,
            auth,
            &key_pair,
            id,
            &failed,
            &task_counter,
            total_tasks,
        )
    }))
    .buffer_unordered(options.parallel);
    pushes.for_each(|r| handle_push_result(r, &failed)).await;
    if failed.load(Ordering::Relaxed) {
        Err(Error::PushFailed)
    } else {
        log::info!("done.");
        Ok(())
    }
}

pub async fn push_initialize_main(
    auth: RegistryAuth,
    options: PushOptions,
    initialize_options: InitializeOptions,
) -> Result<(), Error> {
    let nix_cache_info = build_nix_cache_info(&options, &initialize_options);
    log::debug!("nix-cache-info:\n{nix_cache_info}");
    let key = "nix-cache-info".to_owned();
    let content_type = "text/x-nix-cache-info".to_owned();
    let mut ctx = RegistryOptions::from_push_options(&options).context(auth);
    let location = OciLocation {
        registry: options.registry,
        repository: options.repository,
        key,
    };
    let item = OciItem {
        content_type: Some(content_type),
        data: nix_cache_info.into_bytes(),
    };
    registry::put(&mut ctx, &location, item).await?;
    Ok(())
}
