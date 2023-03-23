pub mod sign;

use std::{
    collections::{HashSet, VecDeque},
    str::FromStr,
};

use nix_base32::to_nix_base32;
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::{error::Error, options::PushOptions};

use self::sign::{NixKeyPair, NixSignatureList};

static STORE_PATH_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new("^([a-z0-9]+)-(.*)$").unwrap());

#[derive(Debug, Clone)]
pub struct NarInfo {
    pub store_path: String,
    pub url: String,
    pub compression: String,
    pub file_hash: String,
    pub file_size: usize,
    pub nar_hash: String,
    pub nar_size: usize,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    pub sig: String,
}

#[derive(Debug, Clone)]
pub struct PathInfo {
    pub id: i64,
    pub path: String,
    pub deriver_store_paths: Option<String>,
    pub nar_size: i64,
    pub sigs: String,
    pub reference_store_paths: Vec<String>,
}

pub fn query_db_ids(
    db: &rusqlite::Connection,
    paths: HashSet<String>,
) -> Result<HashSet<i64>, Error> {
    paths.into_iter().map(|p| query_path_id(db, &p)).collect()
}

pub fn compute_closure(
    db: &rusqlite::Connection,
    ids: HashSet<i64>,
) -> Result<HashSet<i64>, Error> {
    let mut queue: VecDeque<_> = ids.into_iter().collect();
    let mut result = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if result.contains(&id) {
            continue;
        }
        result.insert(id);
        let mut references = VecDeque::from(query_references(db, id)?);
        queue.append(&mut references);
    }
    Ok(result)
}

pub fn query_path_id(db: &rusqlite::Connection, path: &str) -> Result<i64, Error> {
    let mut query_path = db.prepare_cached("SELECT id FROM ValidPaths WHERE path = ?")?;
    let mut query_result = query_path.query(rusqlite::params![path])?;
    let row = match query_result.next()? {
        Some(r) => r,
        None => return Err(Error::NoPathInfo(path.to_owned())),
    };
    let id = row.get::<_, i64>(0)?; // sqlite integer
    if query_result.next()?.is_some() {
        return Err(Error::DuplicatedPathInfo(path.to_owned()));
    }
    Ok(id)
}

pub fn query_references(db: &rusqlite::Connection, id: i64) -> Result<Vec<i64>, Error> {
    let mut query_reference = db.prepare_cached("SELECT reference FROM Refs WHERE referrer = ?")?;
    let query_result = query_reference.query_map(rusqlite::params![id], |row| row.get(0))?;
    let mut result = vec![];
    for r in query_result {
        result.push(r?);
    }
    Ok(result)
}

pub fn store_path_to_hash(options: &PushOptions, store_path: &str) -> Result<String, Error> {
    let stripped = strip_store_dir(options, store_path)?;
    let hash = match STORE_PATH_REGEX.captures(&stripped) {
        Some(captures) => captures[1].to_owned(),
        None => return Err(Error::InvalidStorePath(store_path.to_owned())),
    };
    Ok(hash)
}

pub fn strip_store_dir(options: &PushOptions, store_path: &str) -> Result<String, Error> {
    let prefix = format!("{}/", options.store_dir);
    let stripped = match store_path.strip_prefix(&prefix) {
        None => return Err(Error::InvalidStorePath(store_path.to_owned())),
        Some(s) => s,
    };
    Ok(stripped.to_owned())
}

pub fn filter_id(
    options: &PushOptions,
    key_pair: &NixKeyPair,
    db: &rusqlite::Connection,
    id: i64,
) -> Result<bool, Error> {
    let mut query_sigs = db.prepare_cached("SELECT sigs FROM ValidPaths WHERE id = ?")?;
    let sigs =
        query_sigs.query_row(rusqlite::params![id], |row| row.get::<_, Option<String>>(0))?;
    match sigs {
        None => Ok(false),
        Some(s) => filter_id_single(options, key_pair, &s),
    }
}

pub fn filter_id_single(
    options: &PushOptions,
    key_pair: &NixKeyPair,
    sigs: &str,
) -> Result<bool, Error> {
    let sig_list = NixSignatureList::from_str(sigs)?;
    for sig in sig_list.0 {
        if options.excluded_signing_key_pattern.is_match(&sig.name) {
            log::trace!("excluded path with signature name: {}", sig.name);
            return Ok(false);
        }
        if sig.name == key_pair.name && !options.already_signed {
            log::trace!(
                "excluded already signed path with signature name: {}",
                sig.name
            );
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn query_path_info(db: &rusqlite::Connection, id: i64) -> Result<PathInfo, Error> {
    let mut query_info =
        db.prepare_cached("SELECT path, deriver, narSize, sigs FROM ValidPaths WHERE id = ?")?;
    let (path, deriver_store_paths, nar_size, sigs) = query_info
        .query_row(rusqlite::params![id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
    let mut query_reference_paths = db.prepare_cached(
        "SELECT path FROM ValidPaths WHERE id IN (SELECT reference FROM Refs WHERE referrer = ?)",
    )?;
    let mut reference_store_paths: Vec<_> = query_reference_paths
        .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))?
        .collect::<Result<_, rusqlite::Error>>()?;
    reference_store_paths.sort();
    Ok(PathInfo {
        id,
        path,
        deriver_store_paths,
        nar_size,
        sigs,
        reference_store_paths,
    })
}

pub fn sha256_nix_base32(data: &[u8]) -> String {
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize()
    };
    to_nix_base32(&sha256[..])
}

pub fn build_nar_info(nar_info: NarInfo) -> String {
    let NarInfo {
        store_path,
        url,
        compression,
        file_hash,
        file_size,
        nar_hash,
        nar_size,
        references: references_vec,
        deriver,
        sig,
    } = nar_info;
    let references = references_vec.join(" ");
    format!(
        "StorePath: {store_path}
URL: {url}
Compression: {compression}
FileHash: sha256:{file_hash}
FileSize: {file_size}
NarHash: sha256:{nar_hash}
NarSize: {nar_size}
References: {references}{optional_deriver_line}
Sig: {sig}
",
        optional_deriver_line = match deriver {
            Some(s) => format!("\nDeriver: {s}"),
            None => "".to_owned(),
        }
    )
}

// fingerprintPath: https://github.com/NixOS/nix/blob/master/perl/lib/Nix/Manifest.pm#L234
pub fn nar_info_fingerprint(
    store_dir: &str,
    store_path: &str,
    nar_hash: &str,
    nar_size: usize,
    references: &[String],
) -> String {
    let fingerprint = format!(
        "1;{store_path};sha256:{nar_hash};{nar_size};{comma_delimited_references}",
        comma_delimited_references = references
            .iter()
            .map(|r| format!("{store_dir}/{r}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    log::trace!("fingerprint: {fingerprint}");
    fingerprint
}
