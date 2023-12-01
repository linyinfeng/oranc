use std::path::PathBuf;

use http::{Response, StatusCode};
use hyper::Body;
use oci_distribution::secrets::RegistryAuth;
use reqwest::Url;
use warp::Rejection;

use crate::error::Error;

use super::ServerContext;

pub async fn check_and_redirect(
    ctx: &ServerContext,
    key: &str,
    auth: &RegistryAuth,
) -> Result<Option<Response<Body>>, Rejection> {
    match check(ctx, key, auth).await? {
        Some(url) => Ok(Some(redirect_response(key, &url)?)),
        None => Ok(None),
    }
}

pub async fn check(
    ctx: &ServerContext,
    key: &str,
    auth: &RegistryAuth,
) -> Result<Option<Url>, Error> {
    let max_retry = ctx.options.max_retry;
    if max_retry < 1 {
        return Err(Error::InvalidMaxRetry(max_retry));
    }

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
        for attempt in 1..max_retry {
            let response = ctx
                .http_client
                .head(url.clone())
                .send()
                .await
                .map_err(Error::Reqwest)?;
            if response.status() == StatusCode::OK {
                return Ok(Some(url));
            } else if response.status() == StatusCode::NOT_FOUND {
                break;
            } else {
                log::warn!(
                    "query upstream url '{url}', attempt {attempt}/{max_retry} failed: {:?}",
                    response
                );
            }
        }
    }
    Ok(None)
}

pub fn upstream_url(base: &Url, key: &str) -> Result<Url, Error> {
    let path = base.path();
    let new_path = PathBuf::from(path).join(key);
    match new_path.to_str() {
        Some(p) => {
            let mut upstream = base.clone();
            upstream.set_path(p);
            Ok(upstream)
        }
        None => Err(Error::InvalidPath(new_path)),
    }
}

pub fn redirect_response(key: &str, url: &Url) -> Result<Response<Body>, Error> {
    log::info!("redirect: key = {key}, url = {url}");
    Response::builder()
        .status(StatusCode::FOUND)
        .header(http::header::LOCATION, url.to_string())
        .body(Body::empty())
        .map_err(Error::Http)
}
