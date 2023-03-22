use std::path::PathBuf;

use crate::error::Error;
use crate::registry;
use crate::registry::build_reference;
use crate::registry::get_layer_info;
use crate::registry::LayerInfo;
use bytes::Bytes;

use data_encoding::BASE64;
use http::header;
use http::Response;
use http::StatusCode;
use hyper::Body;

use oci_distribution::Client;
use oci_distribution::{secrets::RegistryAuth, Reference};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Url;
use tokio_util::io::ReaderStream;
use warp::{Filter, Rejection, Reply};

use crate::options::ServerOptions;

const OK_RESPONSE_BODY: &str = "<_/>";
const NO_SUCH_KEY_RESPONSE_BODY: &str = "<Error><Code>NoSuchKey</Code></Error>";

static AUTH_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new("^AWS4-HMAC-SHA256 Credential=([^ /,]+)/.*$").unwrap());
static DECODED_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.+)$").unwrap());

#[derive(Debug, Clone)]
struct ServerContext {
    options: ServerOptions,
    http_client: reqwest::Client,
}

async fn get(
    ctx: ServerContext,
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
    ctx: ServerContext,
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
    ctx: &ServerContext,
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
    _ctx: ServerContext,
    (key, reference): (String, Reference),
    auth: RegistryAuth,
    optional_content_type: Option<String>,
    body: Bytes,
) -> Result<Response<&'static str>, Rejection> {
    log::info!("put: key = {key}, reference = {reference:?}");

    let mut client: Client = Default::default();
    registry::put(
        &mut client,
        &reference,
        &auth,
        &key,
        optional_content_type,
        body.to_vec(),
    )
    .await?;
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

fn reference() -> impl Filter<Extract = ((String, Reference),), Error = Rejection> + Copy {
    warp::path::param() // registry
        .and(warp::path::param()) // repository part1
        .and(warp::path::param()) // repository part1
        .and(warp::path::tail()) // key
        .map(
            |registry, rep1: String, rep2: String, tail: warp::path::Tail| {
                let repository = format!("{rep1}/{rep2}");
                let key = tail.as_str();
                (key.to_owned(), build_reference(registry, repository, key))
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

pub async fn server_main(options: ServerOptions) {
    let http_client = reqwest::Client::new();
    let ctx = ServerContext {
        options,
        http_client,
    };

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
