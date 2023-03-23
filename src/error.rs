use std::{ffi::OsString, path::PathBuf, string::FromUtf8Error};

use http::StatusCode;
use oci_distribution::{errors::OciDistributionError, Reference};
use warp::reject::Reject;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] http::Error),
    #[error("decode error: {0}")]
    Decode(#[from] data_encoding::DecodeError),
    #[error("from utf-8 error: {0}")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("invalid authorization header: {0}")]
    InvalidAuthorization(String),
    #[error("oci distribution error: {0}")]
    OciDistribution(#[from] OciDistributionError),
    #[error("invalid imag`e layer count: {0}")]
    InvalidLayerCount(usize),
    #[error("invalid image layer media type: {0}")]
    InvalidLayerMediaType(String),
    #[error("lack of layer annotations")]
    NoLayerAnnotations,
    #[error("lack of layer annotation key: {0}")]
    NoLayerAnnotationKey(String),
    #[error("reference not found: {0}")]
    ReferenceNotFound(Reference),
    #[error("invalid path: {0:?}")]
    InvalidPath(PathBuf),
    #[error("invalid os string: {0:?}")]
    InvalidOsString(OsString),
    #[error("reqwest error: {0:?}")]
    Reqwest(#[from] reqwest::Error),
    #[error("upstream error: {0:?}")]
    Upstream(reqwest::Response),
    #[error("io error: {0:?}")]
    Io(#[from] std::io::Error),
    #[error("rusqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),
    #[error("duplicated path info: {0}")]
    DuplicatedPathInfo(String),
    #[error("no path info: {0}")]
    NoPathInfo(String),
    #[error("invalid store path: {0}")]
    InvalidStorePath(String),
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
    #[error("invalid max retry number: {0}")]
    InvalidMaxRetry(usize),
    #[error("retry all fails: {0:?}")]
    RetryAllFails(Vec<Error>),
    #[error("nix db folder '{0}' is not writable")]
    NixDbFolderNotWritable(String),
    #[error("push failed")]
    PushFailed,
}

impl Error {
    pub fn code(&self) -> StatusCode {
        match self {
            Error::Http(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Decode(_) => StatusCode::BAD_REQUEST,
            Error::FromUtf8(_) => StatusCode::BAD_REQUEST,
            Error::InvalidAuthorization(_) => StatusCode::BAD_REQUEST,
            Error::OciDistribution(_) => StatusCode::BAD_REQUEST,
            Error::InvalidLayerCount(_) => StatusCode::BAD_REQUEST,
            Error::InvalidLayerMediaType(_) => StatusCode::BAD_REQUEST,
            Error::NoLayerAnnotations => StatusCode::BAD_REQUEST,
            Error::NoLayerAnnotationKey(_) => StatusCode::BAD_REQUEST,
            Error::ReferenceNotFound(_) => StatusCode::NOT_FOUND,
            Error::InvalidPath(_) => StatusCode::BAD_REQUEST,
            Error::InvalidOsString(_) => StatusCode::BAD_REQUEST,
            Error::Reqwest(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Upstream(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Rusqlite(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::DuplicatedPathInfo(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::NoPathInfo(_) => StatusCode::BAD_REQUEST,
            Error::InvalidStorePath(_) => StatusCode::BAD_REQUEST,
            Error::InvalidSignature(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::EarlyStop => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Join(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidNarSize(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::NarSizeNotMatch(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidMaxRetry(_) => StatusCode::BAD_REQUEST,
            Error::RetryAllFails(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::NixDbFolderNotWritable(_) => StatusCode::BAD_REQUEST,
            Error::PushFailed => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Reject for Error {}
