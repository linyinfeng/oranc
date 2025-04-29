use crate::error::Error;
use crate::registry;
use crate::registry::LayerInfo;
use crate::registry::OciItem;
use crate::registry::OciLocation;
use crate::registry::RegistryOptions;
use crate::registry::get_layer_info;

pub mod upstream;

use bytes::Bytes;
use data_encoding::BASE64;
use http::Response;
use http::StatusCode;
use http::header;
use hyper::Body;

use oci_distribution::secrets::RegistryAuth;
use once_cell::sync::Lazy;
use regex::Regex;
use warp::{Filter, Rejection, Reply};

use crate::options::ServerOptions;

const OK_RESPONSE_BODY: &str = "<_/>";
const NO_SUCH_KEY_RESPONSE_BODY: &str = "<Error><Code>NoSuchKey</Code></Error>";

static AWS_AUTH_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new("^AWS4-HMAC-SHA256 Credential=([^ /,]+)/.*$").unwrap());
static BASIC_AUTH_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^Basic (.*)$").unwrap());
static DECODED_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.+)$").unwrap());

#[derive(Debug, Clone)]
pub struct ServerContext {
    options: ServerOptions,
    http_client: reqwest::Client,
}

pub async fn get(
    ctx: ServerContext,
    location: OciLocation,
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    log::info!("get: {location}");
    if let Some(response) = upstream::check_and_redirect(&ctx, &location.key, &auth).await? {
        return Ok(response);
    }
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let LayerInfo {
        reference,
        digest,
        content_type,
    } = get_layer_info(&mut registry_ctx, &location)
        .await?
        .ok_or(Error::ReferenceNotFound(location.clone()))?;
    let blob_stream = registry_ctx
        .client
        .pull_blob_stream(&reference, &digest)
        .await
        .map_err(Error::OciDistribution)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::wrap_stream(blob_stream))
        .map_err(Error::Http)?)
}

pub async fn head(
    ctx: ServerContext,
    location: OciLocation,
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    log::info!("head: {location}");
    if let Some(response) = upstream::check_and_redirect(&ctx, &location.key, &auth).await? {
        return Ok(response);
    }
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let LayerInfo {
        reference: _,
        digest: _,
        content_type,
    } = get_layer_info(&mut registry_ctx, &location)
        .await?
        .ok_or(Error::ReferenceNotFound(location.clone()))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::empty())
        .map_err(Error::Http)?)
}

pub async fn put(
    ctx: ServerContext,
    location: OciLocation,
    auth: RegistryAuth,
    content_type: Option<String>,
    body: Bytes,
) -> Result<Response<&'static str>, Rejection> {
    log::info!("put: {location}");
    // on upstream query for put
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let item = OciItem {
        content_type,
        data: body.to_vec(),
    };
    registry::put(&mut registry_ctx, &location, item).await?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(OK_RESPONSE_BODY)
        .map_err(Error::Http)?)
}

pub fn registry_auth() -> impl Filter<Extract = (RegistryAuth,), Error = Rejection> + Copy {
    warp::header::optional("authorization").and_then(parse_auth)
}

pub async fn parse_auth(opt: Option<String>) -> Result<RegistryAuth, Rejection> {
    match opt {
        None => Ok(RegistryAuth::Anonymous),
        Some(original) => {
            let captures = (BASIC_AUTH_PATTERN.captures(&original))
                .or_else(|| AWS_AUTH_PATTERN.captures(&original));
            let encoded = match &captures {
                Some(c) => c[1].as_bytes(),
                None => return Err(Error::InvalidAuthorization(original).into()),
            };
            let bytes = BASE64.decode(encoded).map_err(Error::Decode)?;
            let decoded = String::from_utf8(bytes).map_err(Error::FromUtf8)?;
            match DECODED_PATTERN.captures(&decoded) {
                Some(captures) => Ok(RegistryAuth::Basic(
                    captures[1].to_string(),
                    captures[2].to_string(),
                )),
                None => Err(Error::InvalidAuthorization(original).into()),
            }
        }
    }
}

pub fn oci_location() -> impl Filter<Extract = (OciLocation,), Error = Rejection> + Copy {
    warp::path::param() // registry
        .and(warp::path::param()) // repository part1
        .and(warp::path::param()) // repository part1
        .and(warp::path::tail()) // key
        .and_then(convert_to_oci_location)
}

pub async fn convert_to_oci_location(
    registry: String,
    rep1: String,
    rep2: String,
    tail: warp::path::Tail,
) -> Result<OciLocation, Rejection> {
    let tail_str = tail.as_str();
    let decoded_registry = urlencoding::decode(&registry).map_err(Error::FromUtf8)?;
    let decoded_rep1 = urlencoding::decode(&rep1).map_err(Error::FromUtf8)?;
    let decoded_rep2 = urlencoding::decode(&rep2).map_err(Error::FromUtf8)?;
    let decoded_tail = urlencoding::decode(tail_str).map_err(Error::FromUtf8)?;
    let repository = format!("{decoded_rep1}/{decoded_rep2}");
    Ok(OciLocation {
        registry: decoded_registry.to_string(),
        repository,
        key: decoded_tail.to_string(),
    })
}

pub async fn handle_error(rejection: Rejection) -> Result<impl Reply, Rejection> {
    log::trace!("handle rejection: {rejection:?}");
    let code;
    let message;
    if let Some(e) = rejection.find::<Error>() {
        log::debug!("handle error: {e}");
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

pub async fn log_rejection(rejection: Rejection) -> Result<Response<Body>, Rejection> {
    log::debug!("unhandled rejection: {rejection:?}");
    Err(rejection)
}

pub async fn server_main(options: ServerOptions) -> Result<(), Error> {
    let http_client = reqwest::Client::new();
    let ctx = ServerContext {
        options,
        http_client,
    };

    let ctx_filter = {
        let ctx = ctx.clone();
        warp::any().map(move || ctx.clone())
    };
    let common = || ctx_filter.clone().and(oci_location()).and(registry_auth());
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
    Ok(())
}
