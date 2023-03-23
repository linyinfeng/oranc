use std::io::{self};
use std::sync::atomic::AtomicUsize;
use std::{
    collections::HashSet,
    env, fs,
    sync::atomic::{AtomicBool, Ordering},
};

use futures::StreamExt;

use oci_distribution::secrets::RegistryAuth;

const NIX_DB_PATH: &str = "/nix/var/nix/db/db.sqlite";
const NAR_CONTENT_TYPE: &str = "application/x-nix-nar";
const NARINFO_CONTENT_TYPE: &str = "text/x-nix-narinfo";

use crate::nix::NarInfo;
use crate::{
    error::Error,
    nix,
    options::{InitializeOptions, PushOptions, PushSubcommands},
    registry::{self, build_reference},
};

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

async fn check_presence(
    options: &PushOptions,
    client: &mut oci_distribution::Client,
    auth: &RegistryAuth,
    nar_info_filename: &str,
) -> Result<bool, Error> {
    let reference = build_reference(
        options.registry.clone(),
        options.repository.clone(),
        nar_info_filename,
    );
    Ok(
        registry::get_layer_info(client, &reference, auth, options.max_retry)
            .await?
            .is_some(),
    )
}

async fn push_one(
    options: &PushOptions,
    auth: &RegistryAuth,
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

    // this function runs in parallel and use its own connections
    let mut client: oci_distribution::Client = Default::default();
    let conn = rusqlite::Connection::open(NIX_DB_PATH)?;

    let path_info = nix::query_path_info(&conn, id)?;
    let store_path = path_info.path.clone();
    let store_path_hash = nix::store_path_to_hash(options, &store_path)?;
    let nar_info_filename = format!("{store_path_hash}.narinfo");

    not_failed()?;
    log::info!("[{task_header}] querying '{}'...", store_path);
    if check_presence(options, &mut client, auth, &nar_info_filename).await? {
        log::debug!("[{task_header}] skipped '{}'.", store_path);
        return Ok(());
    }

    not_failed()?;
    log::info!("[{task_header}] pushing  '{}'...", store_path);
    let (nar_size, nar_hash, nar_file_data, nar_file_size, nar_file_hash) =
        tokio::task::spawn_blocking({
            // clone data
            let zstd_level = options.zstd_level;
            let store_path = store_path.clone();
            move || {
                let mut nar_data = vec![];
                let mut nar_encoder = nix_nar::Encoder::new(store_path);
                io::copy(&mut nar_encoder, &mut nar_data)?;
                let nar_size = nar_data.len();
                let nar_hash = nix::sha256_nix_base32(&nar_data[..]);
                let mut nar_file_data = vec![];
                zstd::stream::copy_encode(&nar_data[..], &mut nar_file_data, zstd_level)?;
                drop(nar_data); // save some memory
                let nar_file_size = nar_file_data.len();
                let nar_file_hash = nix::sha256_nix_base32(&nar_file_data[..]);
                Ok::<_, Error>((
                    nar_size,
                    nar_hash,
                    nar_file_data,
                    nar_file_size,
                    nar_file_hash,
                ))
            }
        })
        .await??;

    let expected_nar_size: usize = path_info
        .nar_size
        .try_into()
        .map_err(Error::InvalidNarSize)?;
    if nar_size != expected_nar_size {
        return Err(Error::NarSizeNotMatch(path_info.nar_size, nar_size));
    }

    let nar_filename = format!("{nar_hash}.nar.zst");
    let nar_file_url = format!("nar/{nar_filename}");
    let nar_oci_reference = build_reference(
        options.registry.to_owned(),
        options.repository.to_owned(),
        &nar_filename,
    );
    let references = path_info
        .reference_store_paths
        .iter()
        .map(|p| nix::strip_store_dir(options, p))
        .collect::<Result<_, _>>()?;
    let deriver = nix::strip_store_dir(options, &path_info.deriver_store_paths)?;
    let nar_info = NarInfo {
        store_path: store_path.clone(),
        url: nar_file_url.clone(),
        compression: "zstd".to_owned(),
        file_hash: nar_file_hash,
        file_size: nar_file_size,
        nar_hash,
        nar_size,
        references,
        deriver,
        sig: path_info.sigs,
    };
    let nar_info_oci_reference = build_reference(
        options.registry.to_owned(),
        options.repository.to_owned(),
        &nar_info_filename,
    );
    let nar_info_content = nix::build_nar_info(nar_info);
    log::debug!("[{task_header}] narinfo:\n{nar_info_content}");
    let nar_info_data = nar_info_content.into_bytes();

    registry::put(
        options,
        &mut client,
        &nar_oci_reference,
        auth,
        &nar_file_url,
        Some(NAR_CONTENT_TYPE.to_owned()),
        nar_file_data,
    )
    .await?;

    registry::put(
        options,
        &mut client,
        &nar_info_oci_reference,
        auth,
        &nar_info_filename,
        Some(NARINFO_CONTENT_TYPE.to_owned()),
        nar_info_data,
    )
    .await?;

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
    let lines = std::io::stdin().lines();
    let mut input_paths = HashSet::new();
    for result in lines {
        let line = result?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let caonoicalized = fs::canonicalize(trimmed)?;
            input_paths.insert(
                caonoicalized
                    .into_os_string()
                    .into_string()
                    .map_err(Error::InvalidOsString)?,
            );
        }
    }
    log::debug!("input paths: {:#?}", input_paths);
    let conn = rusqlite::Connection::open(NIX_DB_PATH)?;
    let id_closures = if options.no_closure {
        nix::query_db_ids(&conn, input_paths)?
    } else {
        let ids = nix::query_db_ids(&conn, input_paths)?;
        nix::compute_closure(&conn, ids)?
    };
    log::debug!("id closure: {:#?}", id_closures);
    let mut filtered = HashSet::new();
    for id in id_closures {
        if nix::filter_id(options, &conn, id)? {
            filtered.insert(id);
        }
    }
    let task_counter = AtomicUsize::new(1);
    let total_tasks = filtered.len();
    let failed = AtomicBool::new(false);
    log::debug!("filtered: {:#?}", filtered);
    let pushes = futures::stream::iter(
        filtered
            .into_iter()
            .map(|id| push_one(options, auth, id, &failed, &task_counter, total_tasks)),
    )
    .buffer_unordered(options.parallel);
    pushes.for_each(|r| handle_push_result(r, &failed)).await;
    Ok(())
}

pub async fn push_initialize_main(
    auth: RegistryAuth,
    options: PushOptions,
    initialize_options: InitializeOptions,
) -> Result<(), Error> {
    let nix_cache_info = build_nix_cache_info(&options, &initialize_options);
    log::debug!("nix-cache-info:\n{nix_cache_info}");
    let key = "nix-cache-info";
    let content_type = "text/x-nix-cache-info";
    let reference = build_reference(options.registry.clone(), options.repository.clone(), key);
    let mut client = Default::default();
    registry::put(
        &options,
        &mut client,
        &reference,
        &auth,
        key,
        Some(content_type.to_string()),
        nix_cache_info.into_bytes(),
    )
    .await?;
    Ok(())
}
