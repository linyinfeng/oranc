use std::io::{self, Read, Write};

use std::sync::atomic::AtomicUsize;
use std::{
    collections::HashSet,
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use futures::{StreamExt, TryStreamExt, future};

use oci_client::secrets::RegistryAuth;
use once_cell::sync::Lazy;
use tempfile::tempdir_in;

const NIX_DB_DIR: &str = "/nix/var/nix/db";
static NIX_DB_FILE: Lazy<String> = Lazy::new(|| format!("{}/db.sqlite", NIX_DB_DIR));
const NAR_CONTENT_TYPE: &str = "application/x-nix-nar";
const NARINFO_CONTENT_TYPE: &str = "text/x-nix-narinfo";
/// Size of the chunks of compressed NAR passed from the encoder to the
/// upload, the same as the chunk size of chunked uploads of `oci_client`.
const NAR_CHUNK_SIZE: usize = 4 * 1024 * 1024;
/// Number of chunks of compressed NAR waiting for upload before the encoder
/// blocks.
const NAR_CHUNK_CAPACITY: usize = 1;

use crate::hashing::HashingWriter;
use crate::nix::sign::{NixKeyPair, NixSignatureList};
use crate::nix::{NarInfo, NixHash, PathInfo};
use crate::registry::{OciBlob, OciItem, OciLocation, RegistryContext, RegistryOptions};
use crate::{
    chunk,
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

/// Returns the location of `key` in the repository to push to.
fn oci_location(options: &PushOptions, key: String) -> OciLocation {
    OciLocation {
        registry: options.registry.clone(),
        repository: options.repository.clone(),
        key,
    }
}

/// Returns the URL, relative to the cache, of the compressed NAR whose
/// uncompressed content has hash `nar_hash`.
fn nar_file_url(nar_hash: &NixHash) -> String {
    format!("nar/{}.nar.zst", nar_hash.base32)
}

/// SHA-256 hashes and sizes of a NAR and of its compressed form.
struct EncodedNar {
    nar_sha256: [u8; 32],
    nar_size: u64,
    file_sha256: [u8; 32],
    file_size: u64,
}

/// Encodes `store_path` as a NAR and writes it compressed with zstd into
/// `sink`, returning `sink` back.
///
/// Fails if the size of the NAR is not `expected_nar_size`.
fn encode_nar<W: Write>(
    store_path: &str,
    expected_nar_size: i64,
    zstd_level: i32,
    sink: W,
) -> Result<(W, EncodedNar), Error> {
    let expected_size = u64::try_from(expected_nar_size).map_err(Error::InvalidNarSize)?;
    let file_writer = HashingWriter::new(sink);
    let mut compressor = zstd::Encoder::new(file_writer, zstd_level)?;
    compressor.set_pledged_src_size(Some(expected_size))?;
    let mut nar_writer = HashingWriter::new(compressor);
    let mut nar_encoder = nix_nar::Encoder::new(store_path)?;
    // Bound what goes into zstd so set_pledged_src_size doesn't fail before we
    // can report NarSizeNotMatch with the full actual size.
    io::copy(
        &mut nar_encoder.by_ref().take(expected_size),
        &mut nar_writer,
    )?;
    let extra_size = io::copy(&mut nar_encoder, &mut io::sink())?;
    let (compressor, nar_size, nar_sha256) = nar_writer.finish();
    let actual_size = nar_size.saturating_add(extra_size);
    if expected_size != actual_size {
        return Err(Error::NarSizeNotMatch(expected_nar_size, actual_size));
    }
    let (sink, file_size, file_sha256) = compressor.finish()?.finish();
    let encoded = EncodedNar {
        nar_sha256,
        nar_size,
        file_sha256,
        file_size,
    };
    Ok((sink, encoded))
}

/// Builds the narinfo of the store path described by `path_info`, whose
/// NAR is `encoded`, and signs it with `key_pair`.
fn build_nar_info(
    options: &PushOptions,
    key_pair: &NixKeyPair,
    path_info: PathInfo,
    encoded: &EncodedNar,
) -> Result<NarInfo, Error> {
    let nar_hash = NixHash::from_sha256(&encoded.nar_sha256);
    let references: Vec<String> = path_info
        .reference_store_paths
        .iter()
        .map(|p| nix::strip_store_dir(options, p))
        .collect::<Result<_, _>>()?;
    let deriver: Option<String> = path_info
        .deriver_store_paths
        .map_or(Ok::<_, Error>(None), |p| {
            Ok(Some(nix::strip_store_dir(options, &p)?))
        })?;
    let fingerprint = nix::nar_info_fingerprint(
        &options.store_dir,
        &path_info.path,
        &nar_hash,
        encoded.nar_size,
        &references,
    );
    let sign = key_pair.sign(fingerprint.as_bytes())?;
    let mut sig_list = NixSignatureList::from_optional_str(&path_info.sigs)?;
    sig_list.merge(key_pair, fingerprint.as_bytes(), sign)?;
    Ok(NarInfo {
        store_path: path_info.path,
        url: nar_file_url(&nar_hash),
        compression: "zstd".to_owned(),
        file_hash: NixHash::from_sha256(&encoded.file_sha256),
        file_size: encoded.file_size,
        nar_hash,
        nar_size: encoded.nar_size,
        references,
        deriver,
        sigs: sig_list,
        ca: path_info.ca,
    })
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

    let path_info = tokio::task::spawn_blocking({
        let options = options.clone();
        move || {
            // this function runs in parallel and use its own connections
            let conn = nix_db_connection(&options)?;
            nix::query_path_info(&conn, id)
        }
    })
    .await??;

    let store_path_hash = nix::store_path_to_hash(options, &path_info.path)?;

    log::info!("[{task_header}] pushing  '{}'...", path_info.path);

    let ctx = RegistryOptions::from_push_options(options).context(auth.clone());

    // push nar first
    let encoded = push_nar(&ctx, options, &path_info).await?;

    let nar_info = build_nar_info(options, key_pair, path_info, &encoded)?;
    let nar_info_content = nar_info.to_string();
    log::debug!("[{task_header}] narinfo:\n{nar_info_content}");
    let nar_info_location = oci_location(options, format!("{store_path_hash}.narinfo"));
    let nar_info_item = OciItem {
        content_type: Some(NARINFO_CONTENT_TYPE.to_owned()),
        blob: OciBlob::from_bytes(nar_info_content.into_bytes()),
    };
    registry::put(&ctx, &nar_info_location, nar_info_item).await
}

/// Encodes the NAR of the store path described by `path_info`, compresses
/// it and pushes it, returning its hashes and sizes.
///
/// The NAR is encoded again on every blob upload attempt instead of being kept
/// for retries.
async fn push_nar(
    ctx: &RegistryContext,
    options: &PushOptions,
    path_info: &PathInfo,
) -> Result<EncodedNar, Error> {
    let description = format!("push NAR blob of '{}'", path_info.path);
    let encoded =
        registry::retry(ctx, &description, || push_nar_blob(ctx, options, path_info)).await?;

    if ctx.options.dry_run {
        return Ok(encoded);
    }

    let digest = registry::sha256_oci_digest(&encoded.file_sha256);
    let size = encoded.file_size;
    let nar_hash = NixHash::from_sha256(&encoded.nar_sha256);
    let location = oci_location(options, nar_file_url(&nar_hash));
    let description = format!("push NAR manifest of '{}'", path_info.path);
    registry::retry(ctx, &description, || {
        registry::push_manifest(
            ctx,
            &location,
            Some(NAR_CONTENT_TYPE.to_owned()),
            &digest,
            size,
        )
    })
    .await?;

    Ok(encoded)
}

/// Makes one attempt of uploading the encoded NAR blob for [`push_nar`].
async fn push_nar_blob(
    ctx: &RegistryContext,
    options: &PushOptions,
    path_info: &PathInfo,
) -> Result<EncodedNar, Error> {
    let (writer, chunks) = chunk::channel(NAR_CHUNK_SIZE, NAR_CHUNK_CAPACITY);
    let encoding = tokio::task::spawn_blocking({
        let store_path = path_info.path.clone();
        let nar_size = path_info.nar_size;
        let zstd_level = options.zstd_level;
        move || {
            let (writer, encoded) = encode_nar(&store_path, nar_size, zstd_level, writer)?;
            writer.finish()?;
            Ok::<_, Error>(encoded)
        }
    });
    let uploading = async {
        let chunks = chunks.map_err(io::Error::other);
        if ctx.options.dry_run {
            log::debug!("dry run, skipped");
            chunks.try_for_each(|_| future::ok(())).await?;
            return Ok(None);
        }
        let blob = registry::push_blob_stream(ctx, &options.registry, &options.repository, chunks);
        Ok(Some(blob.await?))
    };
    let (encoded, uploaded) = tokio::join!(encoding, uploading);
    let (encoded, uploaded) = combine_stream_results(encoded?, uploaded)?;

    if let Some(uploaded) = uploaded {
        let (digest, size) = (&uploaded.blob_digest, uploaded.size);
        let expected_digest = registry::sha256_oci_digest(&encoded.file_sha256);
        if *digest != expected_digest || size != encoded.file_size {
            return Err(Error::UploadedBlobNotMatch(
                expected_digest,
                encoded.file_size,
                digest.clone(),
                size,
            ));
        }
    }
    Ok(encoded)
}

/// Combines the results of writing to a [`chunk::channel`] and consuming its
/// stream concurrently, returning the error that stopped them if any.
fn combine_stream_results<P, C>(
    produced: Result<P, Error>,
    consumed: Result<C, Error>,
) -> Result<(P, C), Error> {
    match (produced, consumed) {
        (Ok(produced), Ok(consumed)) => Ok((produced, consumed)),
        // writing stopped because consuming failed and dropped the stream
        (Err(Error::Io(e)), Err(consumer_error)) if chunk::is_stream_dropped(&e) => {
            Err(consumer_error)
        }
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
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
            input_paths.insert(trimmed.to_string());
        }
    }
    log::info!("number of input paths: {}", input_paths.len());
    log::trace!("input paths: {:#?}", input_paths);
    let canonical_paths: HashSet<_> = input_paths
        .into_iter()
        .map(|p| nix::canonicalize_store_path_input(&options.store_dir, &p))
        .collect::<Result<_, _>>()?;
    log::info!("number of input store paths: {}", canonical_paths.len());
    log::trace!("canonical paths: {:#?}", canonical_paths);
    let id_closures = if options.no_closure {
        nix::query_db_ids(&conn, canonical_paths)?
    } else {
        let ids = nix::query_db_ids(&conn, canonical_paths)?;
        nix::compute_closure(&conn, ids)?
    };
    log::info!("number of store paths in closure: {}", id_closures.len());
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
    log::info!("number of store paths after filtering: {}", filtered.len());
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
    let ctx = RegistryOptions::from_push_options(&options).context(auth);
    let location = oci_location(&options, key);
    let item = OciItem {
        content_type: Some(content_type),
        blob: OciBlob::from_bytes(nix_cache_info.into_bytes()),
    };
    registry::put(&ctx, &location, item).await?;
    Ok(())
}
