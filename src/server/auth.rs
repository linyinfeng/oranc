use std::ops::{Deref, DerefMut};

use axum::extract::FromRequestParts;
use data_encoding::BASE64;
use http::{header::AUTHORIZATION, request::Parts};
use oci_client::secrets::RegistryAuth;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::error::Error;

static AWS_AUTH_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new("^AWS4-HMAC-SHA256 Credential=([^ /,]+)/.*$").unwrap());
static BASIC_AUTH_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^Basic (.*)$").unwrap());
static DECODED_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.+)$").unwrap());

/// A wrapper around OCI RegistryAuth
#[derive(Debug, Clone)]
pub struct Auth(pub RegistryAuth);

impl From<RegistryAuth> for Auth {
    fn from(value: RegistryAuth) -> Self {
        Self(value)
    }
}

impl Deref for Auth {
    type Target = RegistryAuth;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Auth {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<S> FromRequestParts<S> for Auth
where
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S, // authentication are stateless
    ) -> Result<Self, Self::Rejection> {
        let authorization = parts.headers.get(AUTHORIZATION);

        match authorization {
            None => Ok(RegistryAuth::Anonymous.into()),
            Some(value) => {
                let s = value
                    .to_str()
                    .map_err(|e| Error::HeaderToStr(AUTHORIZATION, e))?;
                let captures =
                    (BASIC_AUTH_PATTERN.captures(s)).or_else(|| AWS_AUTH_PATTERN.captures(s));
                let encoded = match &captures {
                    Some(c) => c[1].as_bytes(),
                    None => return Err(Error::InvalidAuthorization(s.to_string())),
                };
                let bytes = BASE64.decode(encoded).map_err(Error::Decode)?;
                let decoded = String::from_utf8(bytes).map_err(Error::FromUtf8)?;
                match DECODED_PATTERN.captures(&decoded) {
                    Some(captures) => Ok(RegistryAuth::Basic(
                        captures[1].to_string(),
                        captures[2].to_string(),
                    )
                    .into()),
                    None => Err(Error::InvalidAuthorization(s.to_string())),
                }
            }
        }
    }
}
