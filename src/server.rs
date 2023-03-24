use crate::error::Error;
use crate::registry::get_layer_info;
use crate::registry::LayerInfo;
use crate::registry::OciLocation;

use crate::registry::RegistryOptions;

use data_encoding::BASE64;
use http::header;
use http::Response;
use http::StatusCode;
use hyper::Body;

use oci_distribution::secrets::RegistryAuth;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio_util::io::ReaderStream;
use warp::{Filter, Rejection, Reply};

use crate::options::ServerOptions;

const NO_SUCH_KEY_RESPONSE_BODY: &str = "<Error><Code>NoSuchKey</Code></Error>";

static AUTH_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new("^AWS4-HMAC-SHA256 Credential=([^ /,]+)/.*$").unwrap());
static DECODED_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.+)$").unwrap());

#[derive(Debug, Clone)]
struct ServerContext {
    options: ServerOptions,
}

async fn get(
    ctx: ServerContext,
    location: OciLocation,
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    log::info!("get: {location}");
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let LayerInfo {
        digest,
        content_type,
    } = get_layer_info(&mut registry_ctx, &location)
        .await?
        .ok_or(Error::ReferenceNotFound(location.clone()))?;
    let blob = registry_ctx
        .client
        .async_pull_blob(&location.reference(), &digest)
        .await
        .map_err(Error::OciDistribution)?;
    let blob_stream = ReaderStream::new(blob);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::wrap_stream(blob_stream))
        .map_err(Error::Http)?)
}

async fn head(
    ctx: ServerContext,
    location: OciLocation,
    auth: RegistryAuth,
) -> Result<Response<Body>, Rejection> {
    log::info!("head: {location}");
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let LayerInfo {
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

fn oci_location() -> impl Filter<Extract = (OciLocation,), Error = Rejection> + Copy {
    warp::path::param() // registry
        .and(warp::path::param()) // repository part1
        .and(warp::path::param()) // repository part1
        .and(warp::path::tail()) // key
        .map(
            |registry, rep1: String, rep2: String, tail: warp::path::Tail| {
                let repository = format!("{rep1}/{rep2}");
                let key = tail.as_str();
                OciLocation {
                    registry,
                    repository,
                    key: key.to_owned(),
                }
            },
        )
}

async fn handle_error(rejection: Rejection) -> Result<impl Reply, Rejection> {
    log::trace!("handle rejection: {rejection:?}");
    let code;
    let message;
    if let Some(e) = rejection.find::<Error>() {
        log::info!("handle error: {e}");
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

pub async fn server_main(options: ServerOptions) -> Result<(), Error> {
    let ctx = ServerContext { options };

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
