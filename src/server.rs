use std::sync::Arc;

use crate::error::Error;
use crate::registry;
use crate::registry::LayerInfo;
use crate::registry::OciItem;
use crate::registry::OciLocation;
use crate::registry::RegistryOptions;
use crate::registry::get_layer_info;
use crate::server::auth::Auth;

pub mod auth;
pub mod upstream;

use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::routing::head;
use axum::routing::put;
use axum_extra::TypedHeader;
use axum_extra::headers::ContentType;
use http::StatusCode;
use http::header;

use crate::options::ServerOptions;

const OK_RESPONSE_BODY: &str = "<_/>";

#[derive(Debug, Clone)]
pub struct ServerContext {
    pub options: ServerOptions,
    pub http_client: reqwest::Client,
}

pub async fn server_main(options: ServerOptions) -> Result<(), Error> {
    let http_client = reqwest::Client::new();
    let ctx = Arc::new(ServerContext {
        options,
        http_client,
    });

    let app = Router::new()
        .route("/", get(async || "oranc: OCI Registry As Nix Cache"))
        .route("/{*path}", get(get_key))
        .route("/{*path}", head(head_key))
        .route("/{*path}", put(put_key))
        .with_state(ctx.clone());

    let listener = tokio::net::TcpListener::bind(&ctx.options.listen)
        .await
        .unwrap();
    log::info!("listening on {:?}", ctx.options.listen);
    Ok(axum::serve(listener, app).await?)
}

async fn get_key(
    State(ctx): State<Arc<ServerContext>>,
    location: OciLocation,
    Auth(auth): Auth,
) -> Result<Response<Body>, Error> {
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
        .pull_blob_stream(&reference, digest.as_str())
        .await
        .map_err(Error::OciDistribution)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from_stream(blob_stream))
        .map_err(Error::Http)
}

async fn head_key(
    State(ctx): State<Arc<ServerContext>>,
    location: OciLocation,
    Auth(auth): Auth,
) -> Result<Response<Body>, Error> {
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
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::empty())
        .map_err(Error::Http)
}

async fn put_key(
    State(ctx): State<Arc<ServerContext>>,
    location: OciLocation,
    Auth(auth): Auth,
    content_type: Option<TypedHeader<ContentType>>,
    body: Bytes,
) -> Result<Response<Body>, Error> {
    log::info!("put: {location}");
    // on upstream query for put
    let mut registry_ctx = RegistryOptions::from_server_options(&ctx.options).context(auth);
    let item = OciItem {
        content_type: content_type.map(|TypedHeader(typ)| typ.to_string()),
        data: body.to_vec(),
    };
    registry::put(&mut registry_ctx, &location, item).await?;
    Response::builder()
        .status(StatusCode::OK)
        .body(OK_RESPONSE_BODY.into()) // s3 client will parse the body
        .map_err(Error::Http)
}
