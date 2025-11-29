use std::{env::VarError, ffi::OsString, string::FromUtf8Error};

use axum::{
    extract::rejection::{PathRejection, QueryRejection},
    response::{IntoResponse, Response},
};
use http::{HeaderName, StatusCode, header::ToStrError};
use oci_client::errors::OciDistributionError;
use reqwest::Url;

use crate::registry::OciLocation;

const NO_SUCH_KEY_RESPONSE_BODY: &str = "<Error><Code>NoSuchKey</Code></Error>";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    // server side errors
    #[error("http error: {0}")]
    Http(#[from] http::Error),
    #[error("reqwest error: {0:?}")]
    Reqwest(#[from] reqwest::Error),
    #[error("io error: {0:?}")]
    Io(#[from] std::io::Error),
    #[error("rusqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),
    #[error("duplicated path info: {0}")]
    DuplicatedPathInfo(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("early stop")]
    EarlyStop,
    #[error("tokio join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("invalid nar size: {0}")]
    InvalidNarSize(<i64 as TryInto<usize>>::Error),
    #[error("nar size not match: expected = {0}, actual = {1}")]
    NarSizeNotMatch(i64, usize),
    #[error("retry all fails: {0:?}")]
    RetryAllFails(Vec<Error>),
    #[error("push failed")]
    PushFailed,
    #[error("nar error: {0}")]
    Nar(#[from] nix_nar::NarError),
    #[error("path rejected in extraction: {0:?}")]
    PathRejection(#[from] PathRejection),
    #[error("upstream url '{0}' can not be base")]
    UpstreamCanNotBeBase(Url),

    // client side errors
    #[error("decode error: {0}")]
    Decode(#[from] data_encoding::DecodeError),
    #[error("tag-to-key error: {0:?}")]
    TagToKey(Vec<Error>),
    #[error("invalid tag error: {0}")]
    InvalidTag(String),
    #[error("from utf-8 error: {0}")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("failed to convert header {0} to string: {1}")]
    HeaderToStr(HeaderName, ToStrError),
    #[error("invalid authorization header: {0}")]
    InvalidAuthorization(String),
    #[error("oci distribution error: {0}")]
    OciDistribution(#[from] OciDistributionError),
    #[error("invalid image layer count: {0}")]
    InvalidLayerCount(usize),
    #[error("invalid image layer media type: {0}")]
    InvalidLayerMediaType(String),
    #[error("lack of layer annotations")]
    NoLayerAnnotations,
    #[error("lack of layer annotation key: {0}")]
    NoLayerAnnotationKey(String),
    #[error("reference not found: {0}")]
    ReferenceNotFound(OciLocation),
    #[error("ill-formed path: {0}")]
    IllFormedPath(String),
    #[error("invalid query string: {0}")]
    InvalidQueryString(#[from] QueryRejection),
    #[error("invalid os string: {0:?}")]
    InvalidOsString(OsString),
    #[error("no path info: {0}")]
    NoPathInfo(String),
    #[error("invalid store path: {0}")]
    InvalidStorePath(String),
    #[error("invalid signing key: {0}")]
    InvalidSigningKey(String),
    #[error("invalid max retry number: {0}")]
    InvalidMaxRetry(usize),
    #[error("nix db folder '{0}' is not writable")]
    NixDbFolderNotWritable(String),
    #[error("ed25519 error: {0}")]
    Ed25519(#[from] ed25519_compact::Error),
    #[error("unable to read environment variable `ORANC_SIGNING_KEY`: {0}")]
    InvalidSigningKeyEnv(VarError),
    #[error("signature mismatch for key '{name}': new = '{new}', exists = '{exists}'")]
    SignatureMismatch {
        name: String,
        new: String,
        exists: String,
    },
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        log::info!("report error: {self}");
        let code = self.code();
        let body = if let Error::ReferenceNotFound(_) = self {
            NO_SUCH_KEY_RESPONSE_BODY.to_string() // s3 client will parse the body
        } else if code.is_client_error() {
            self.to_string()
        } else {
            code.canonical_reason()
                .unwrap_or("unknown error")
                .to_owned()
        };
        (code, body).into_response()
    }
}

impl Error {
    pub fn code(&self) -> StatusCode {
        match self {
            Error::Http(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Reqwest(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Rusqlite(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::DuplicatedPathInfo(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidSignature(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::EarlyStop => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Join(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidNarSize(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::NarSizeNotMatch(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::RetryAllFails(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::PushFailed => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Nar(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::PathRejection(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::UpstreamCanNotBeBase(_) => StatusCode::BAD_REQUEST,

            Error::Decode(_) => StatusCode::BAD_REQUEST,
            Error::TagToKey(_) => StatusCode::BAD_REQUEST,
            Error::InvalidTag(_) => StatusCode::BAD_REQUEST,
            Error::FromUtf8(_) => StatusCode::BAD_REQUEST,
            Error::HeaderToStr(_, _) => StatusCode::BAD_REQUEST,
            Error::InvalidAuthorization(_) => StatusCode::BAD_REQUEST,
            Error::OciDistribution(_) => StatusCode::BAD_REQUEST,
            Error::InvalidLayerCount(_) => StatusCode::BAD_REQUEST,
            Error::InvalidLayerMediaType(_) => StatusCode::BAD_REQUEST,
            Error::NoLayerAnnotations => StatusCode::BAD_REQUEST,
            Error::NoLayerAnnotationKey(_) => StatusCode::BAD_REQUEST,
            Error::ReferenceNotFound(_) => StatusCode::NOT_FOUND,
            Error::IllFormedPath(_) => StatusCode::NOT_FOUND,
            Error::InvalidQueryString(_) => StatusCode::BAD_REQUEST,
            Error::InvalidOsString(_) => StatusCode::BAD_REQUEST,
            Error::NoPathInfo(_) => StatusCode::BAD_REQUEST,
            Error::InvalidStorePath(_) => StatusCode::BAD_REQUEST,
            Error::InvalidSigningKey(_) => StatusCode::BAD_REQUEST,
            Error::InvalidMaxRetry(_) => StatusCode::BAD_REQUEST,
            Error::NixDbFolderNotWritable(_) => StatusCode::BAD_REQUEST,
            Error::Ed25519(_) => StatusCode::BAD_REQUEST,
            Error::InvalidSigningKeyEnv(_) => StatusCode::BAD_REQUEST,
            Error::SignatureMismatch { .. } => StatusCode::BAD_REQUEST,
        }
    }
}
