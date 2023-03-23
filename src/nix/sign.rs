use std::{fmt, str::FromStr};

use data_encoding::BASE64;
use ed25519_compact::{KeyPair, SecretKey, Signature};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::error::Error;

pub static SIG_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new("^([^:]+):(.*)$").unwrap());

#[derive(Debug, Clone)]
pub struct NixKeyPair {
    pub name: String,
    pub key_pair: KeyPair,
}

#[derive(Debug, Clone)]
pub struct NixSignature {
    pub name: String,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct NixSignatureList(pub Vec<NixSignature>);

impl NixKeyPair {
    pub fn from_secret_key_str(s: &str) -> Result<NixKeyPair, Error> {
        let c = SIG_REGEX
            .captures(s)
            .ok_or(Error::InvalidSigningKey(s.to_owned()))?;
        let name = c[1].to_owned();
        let sk_bytes = BASE64.decode(c[2].as_bytes())?;
        let sk = SecretKey::from_slice(&sk_bytes)?;
        let pk = sk.public_key();
        Ok(NixKeyPair {
            name,
            key_pair: KeyPair { sk, pk },
        })
    }

    pub fn sign(&self, data: &[u8]) -> Result<NixSignature, Error> {
        let sign = self.key_pair.sk.sign(data, None);
        let encoded_sign = BASE64.encode(sign.as_ref());
        Ok(NixSignature {
            name: self.name.clone(),
            signature: encoded_sign,
        })
    }

    pub fn verify(&self, data: &[u8], signature: &NixSignature) -> Result<(), Error> {
        Ok(self.key_pair.pk.verify(data, &signature.signature()?)?)
    }
}

impl FromStr for NixSignature {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let c = SIG_REGEX
            .captures(s)
            .ok_or(Error::InvalidSignature(s.to_owned()))?;
        Ok(Self {
            name: c[1].to_owned(),
            signature: c[2].to_owned(),
        })
    }
}

impl NixSignature {
    fn signature(&self) -> Result<Signature, Error> {
        Ok(Signature::from_slice(
            &BASE64.decode(self.signature.as_bytes())?,
        )?)
    }
}

impl fmt::Display for NixSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.name, self.signature)
    }
}

impl FromStr for NixSignatureList {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let s = s
            .split(' ')
            .map(NixSignature::from_str)
            .collect::<Result<_, _>>()?;
        Ok(Self(s))
    }
}

impl NixSignatureList {
    pub fn merge(
        &mut self,
        key_pair: &NixKeyPair,
        data: &[u8],
        new: NixSignature,
    ) -> Result<(), Error> {
        assert!(key_pair.name == new.name);

        let mut already_exists = false;
        for s in self.0.iter() {
            if s.name == new.name {
                key_pair.verify(data, s)?;
                already_exists = true;
                if s.signature != new.signature {
                    return Err(Error::SignatureMismatch {
                        name: key_pair.name.clone(),
                        new: new.signature,
                        exists: s.signature.clone(),
                    });
                }
            }
        }
        if !already_exists {
            self.0.push(new)
        }
        Ok(())
    }
}

impl fmt::Display for NixSignatureList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for s in &self.0 {
            if !first {
                write!(f, " ")?;
            }
            first = false;
            write!(f, "{}", s)?;
        }
        Ok(())
    }
}
