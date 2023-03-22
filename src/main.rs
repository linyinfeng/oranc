mod error;
mod options;

use std::path::PathBuf;

use bytes::Bytes;
use clap::Parser;
use data_encoding::BASE32_DNSSEC;
use data_encoding::BASE64;
use error::Error;
use http::header;
use http::Response;
use http::StatusCode;
use hyper::Body;
use maplit::hashmap;
use oci_distribution::client::Config;
use oci_distribution::client::ImageLayer;
use oci_distribution::config::Architecture;
use oci_distribution::config::ConfigFile;
use oci_distribution::config::Os;
use oci_distribution::config::Rootfs;
use oci_distribution::manifest::OciImageManifest;
use oci_distribution::Client;
use oci_distribution::{secrets::RegistryAuth, Reference};
use once_cell::sync::Lazy;
use options::Options;
use regex::Regex;
use reqwest::Url;
use tokio_util::io::ReaderStream;
use warp::{Filter, Rejection, Reply};

const LAYER_MEDIA_TYPE: &str = "application/octet-stream";
const CONTENT_TYPE_ANNOTATION: &str = "com.linyinfeng.oranc.content.type";
const OK_RESPONSE_BODY: &str = "<_/>";
const NO_SUCH_KEY_RESPONSE_BODY: &str = "<Error><Code>NoSuchKey</Code></Error>";

static AUTH_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new("^AWS4-HMAC-SHA256 Credential=([^ /,]+)/.*$").unwrap());
static DECODED_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.+)$").unwrap());

#[derive(Debug, Clone)]
struct Context {
    options: Options,
    http_client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct LayerInfo {
    digest: String,
    content_type: String,
}

async fn get_layer_info(
    client: &mut Client,
    reference: &Reference,
    auth: &RegistryAuth,
) -> Result<Option<LayerInfo>, Rejection> {
    let (manifest, _hash) = match client.pull_image_manifest(reference, auth).await {
        Ok(t) => t,
        Err(e) => {
            log::trace!("failed to get layer info: {e}");
            return Ok(None);
        }
    };
    match manifest.layers.len() {
        1 => (),
        other => return Err(Error::InvalidLayerCount(other).into()),
    }
    let layer_manifest = &manifest.layers[0];
    if layer_manifest.media_type != LAYER_MEDIA_TYPE {
        return Err(Error::InvalidLayerMediaType(layer_manifest.media_type.clone()).into());
    }
    let annotations = match &layer_manifest.annotations {
        Some(a) => a,
        None => return Err(Error::NoLayerAnnotations.into()),
    };
    let content_type = match annotations.get(CONTENT_TYPE_ANNOTATION) {
        Some(a) => a,
        None => {
            return Err(Error::NoLayerAnnotationKey(CONTENT_TYPE_ANNOTATION.to_string()).into())
        }
    };
    let info = LayerInfo {
        digest: layer_manifest.digest.clone(),
        content_type: content_type.clone(),
    };
    Ok(Some(info))
}

async fn get(
    ctx: Context,
    (key, reference): (String, Reference),
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    match check_upstream(&ctx, &key, &auth).await? {
        None => {
            log::info!("get: key = {key}, reference = {reference:?}");
            let mut client: Client = Default::default();
            let LayerInfo {
                digest,
                content_type,
            } = get_layer_info(&mut client, &reference, &auth)
                .await?
                .ok_or(Error::ReferenceNotFound(reference.clone()))?;
            let blob = client
                .async_pull_blob(&reference, &digest)
                .await
                .map_err(Error::OciDistribution)?;
            let blob_stream = ReaderStream::new(blob);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::wrap_stream(blob_stream))
                .map_err(Error::Http)?)
        }
        Some(url) => redirect_response(&key, &url),
    }
}

async fn head(
    ctx: Context,
    (key, reference): (String, Reference),
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    match check_upstream(&ctx, &key, &auth).await? {
        None => {
            log::info!("head: key = {key}, reference = {reference:?}");
            let mut client: Client = Default::default();
            let LayerInfo {
                digest: _,
                content_type,
            } = get_layer_info(&mut client, &reference, &auth)
                .await?
                .ok_or(warp::reject::not_found())?;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::empty())
                .map_err(Error::Http)?)
        }
        Some(url) => redirect_response(&key, &url),
    }
}

async fn check_upstream(
    ctx: &Context,
    key: &str,
    auth: &RegistryAuth,
) -> Result<Option<Url>, Rejection> {
    if let RegistryAuth::Anonymous = auth {
        // skip check upstream caches if `--upstream-anonymous` is off
        if !ctx.options.upstream_anonymous {
            log::debug!("skipped checking upstream for key: '{}'", key);
            return Ok(None);
        }
    }
    if ctx.options.ignore_upstream.is_match(key) {
        return Ok(None);
    }
    for upstream in &ctx.options.upstream {
        let url = upstream_url(upstream, key)?;
        let response = ctx
            .http_client
            .head(url.clone())
            .send()
            .await
            .map_err(Error::Reqwest)?;
        if response.status() == StatusCode::OK {
            return Ok(Some(url));
        } else if response.status() == StatusCode::NOT_FOUND {
            continue;
        } else {
            return Err(Error::Upstream(response).into());
        }
    }
    Ok(None)
}

fn upstream_url(base: &Url, key: &str) -> Result<Url, Rejection> {
    let path = base.path();
    let new_path = PathBuf::from(path).join(key);
    match new_path.to_str() {
        Some(p) => {
            let mut upstream = base.clone();
            upstream.set_path(p);
            Ok(upstream)
        }
        None => Err(Error::InvalidPath(new_path).into()),
    }
}

fn redirect_response(key: &str, url: &Url) -> Result<Response<Body>, Rejection> {
    log::info!("redirect: key = {key}, url = {url}");
    Ok(Response::builder()
        .status(StatusCode::FOUND)
        .header(http::header::LOCATION, url.to_string())
        .body(Body::empty())
        .map_err(Error::Http)?)
}

async fn put(
    _ctx: Context,
    (key, reference): (String, Reference),
    auth: RegistryAuth,
    optional_content_type: Option<String>,
    body: Bytes,
) -> Result<Response<&'static str>, Rejection> {
    log::info!("put: key = {key}, reference = {reference:?}");

    let mut client: Client = Default::default();

    let content_type = match optional_content_type {
        None => "application/octet-stream".to_string(),
        Some(c) => c,
    };
    let layer_annotations = hashmap! {
        CONTENT_TYPE_ANNOTATION.to_string() => content_type,
    };
    let layer = ImageLayer::new(
        body.to_vec(),
        LAYER_MEDIA_TYPE.to_string(),
        Some(layer_annotations),
    );
    let layer_digest = layer.sha256_digest();
    let layers = vec![layer];

    let rootfs = Rootfs {
        r#type: "layers".to_string(),
        diff_ids: vec![
            // just use layer digest
            layer_digest,
        ],
    };
    let config_file = ConfigFile {
        created: None,
        author: None,
        architecture: Architecture::None,
        os: Os::None,
        config: None,
        rootfs,
        history: vec![],
    };
    let config_annotations = None;
    let config = Config::oci_v1_from_config_file(config_file, config_annotations)
        .map_err(Error::OciDistribution)?;

    let image_annotations = hashmap! {
        "com.linyinfeng.oranc.key".to_string() => key.clone(),
        "org.opencontainers.image.description".to_string() => key.clone(),
    };
    let image_manifest = OciImageManifest::build(&layers, &config, Some(image_annotations));
    client
        .push(&reference, &layers, config, &auth, Some(image_manifest))
        .await
        .map_err(Error::OciDistribution)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(OK_RESPONSE_BODY)
        .map_err(Error::Http)?)
}

fn registry_auth() -> impl Filter<Extract = (RegistryAuth,), Error = Rejection> + Copy {
    warp::header::optional("authorization").and_then(parse_auth)
}

async fn parse_auth(opt: Option<String>) -> Result<RegistryAuth, Rejection> {
    match opt {
        None => Ok(RegistryAuth::Anonymous),
        Some(original) => match AUTH_PATTERN.captures(&original) {
            Some(captures) => {
                let bytes = BASE64
                    .decode(captures[1].as_bytes())
                    .map_err(Error::Decode)?;
                let decoded = String::from_utf8(bytes).map_err(Error::FromUtf8)?;
                match DECODED_PATTERN.captures(&decoded) {
                    Some(captures) => Ok(RegistryAuth::Basic(
                        captures[1].to_string(),
                        captures[2].to_string(),
                    )),
                    None => Err(Error::InvalidAuthorization(original).into()),
                }
            }
            None => Err(Error::InvalidAuthorization(original).into()),
        },
    }
}

fn key_to_tag(key: &str) -> String {
    // https://docs.rs/data-encoding/latest/data_encoding/constant.BASE32_DNSSEC.html
    // It uses a base32 extended hex alphabet.
    // It is case-insensitive when decoding and uses lowercase when encoding.
    // It does not use padding.
    BASE32_DNSSEC.encode(key.as_bytes())
}

fn reference() -> impl Filter<Extract = ((String, Reference),), Error = Rejection> + Copy {
    warp::path::param() // registry
        .and(warp::path::param()) // repository part1
        .and(warp::path::param()) // repository part1
        .and(warp::path::tail()) // key
        .map(
            |registry, rep1: String, rep2: String, tail: warp::path::Tail| {
                let repository = format!("{rep1}/{rep2}");
                let key = tail.as_str();
                let tag = key_to_tag(key);
                log::debug!("key '{key}' to tag '{tag}'");
                (
                    key.to_string(),
                    Reference::with_tag(registry, repository, tag),
                )
            },
        )
}

async fn handle_error(rejection: Rejection) -> Result<impl Reply, Rejection> {
    log::trace!("handle rejection: {rejection:?}");
    let code;
    let message;
    if let Some(e) = rejection.find::<Error>() {
        log::info!("handle error: {e:?}");
        code = e.code();
        match e {
            // otherwise aws clients can not decode 404 error message
            Error::ReferenceNotFound(_) => message = NO_SUCH_KEY_RESPONSE_BODY.to_string(),
            _ => message = format!("error: {}\n", e),
        }
    } else if rejection.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "not found".to_string();
    } else {
        return Err(rejection);
    }
    Ok(warp::reply::with_status(message, code))
}

async fn log_rejection(rejection: Rejection) -> Result<Response<Body>, Rejection> {
    log::debug!("unhandled rejection: {rejection:?}");
    Err(rejection)
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let options = options::Options::parse();
    let http_client = reqwest::Client::new();
    let ctx = Context {
        options,
        http_client,
    };
    log::info!("options = {:#?}", ctx.options);

    let ctx_filter = {
        let ctx = ctx.clone();
        warp::any().map(move || ctx.clone())
    };
    let common = || ctx_filter.clone().and(reference()).and(registry_auth());
    let main = warp::get()
        .and(warp::path::end())
        .map(|| "oranc: OCI Registry As Nix Cache")
        .or(warp::get()
            .and(common())
            .and_then(get)
            .recover(handle_error))
        .or(warp::head()
            .and(common())
            .and_then(head)
            .recover(handle_error))
        .or(warp::put()
            .and(common())
            .and(warp::header::optional("content-type"))
            .and(warp::body::bytes())
            .and_then(put)
            .recover(handle_error));

    let log = warp::log::custom(|info| {
        log::trace!(
            "from {remote_addr:?} {elapsed:?}
{version:?} {method} {host:?} {path} {status}
{request_headers:?}",
            remote_addr = info.remote_addr(),
            elapsed = info.elapsed(),
            version = info.version(),
            method = info.method(),
            host = info.host(),
            path = info.path(),
            status = info.status(),
            request_headers = info.request_headers(),
        )
    });

    let routes = main.recover(log_rejection).with(log);

    warp::serve(routes).run(ctx.options.listen).await;
}
