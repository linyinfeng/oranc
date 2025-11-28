use axum::{body::Body, response::Response};
use http::StatusCode;
use oci_client::secrets::RegistryAuth;
use reqwest::Url;

use crate::error::Error;

use super::ServerContext;

pub async fn check_and_redirect(
    ctx: &ServerContext,
    key: &str,
    auth: &RegistryAuth,
) -> Result<Option<Response<Body>>, Error> {
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
            let response = ctx.http_client.head(url.clone()).send().await?;
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
    let mut upstream = base.clone();
    {
        let mut segments = upstream
            .path_segments_mut()
            .map_err(|_| Error::UpstreamCanNotBeBase(base.clone()))?;
        segments.push(key);
    }
    Ok(upstream)
}

pub fn redirect_response(key: &str, url: &Url) -> Result<Response<Body>, Error> {
    log::info!("redirect: key = {key}, url = {url}");
    Ok(Response::builder()
        .status(StatusCode::FOUND)
        .header(http::header::LOCATION, url.to_string())
        .body(Body::empty())?)
}
