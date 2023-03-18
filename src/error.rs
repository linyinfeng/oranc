use std::{path::PathBuf, string::FromUtf8Error};

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
    #[error("invalid image layer count: {0}")]
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
    #[error("reqwest error: {0:?}")]
    Reqwest(reqwest::Error),
    #[error("upstream error: {0:?}")]
    Upstream(reqwest::Response),
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
            Error::Reqwest(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Upstream(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Reject for Error {}
