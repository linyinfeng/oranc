use clap::Parser;
use clap::ValueEnum;
use data_encoding::BASE32_DNSSEC;
use once_cell::sync::Lazy;
use std::collections::BTreeMap;

use crate::error::Error;

#[derive(Clone, Debug, Parser)]
pub struct EncodingOptions {
    #[arg(long, value_enum, default_value = "custom")]
    pub tag_encoding: TagEncoding,
    #[arg(long)]
    pub fallback_encodings: Vec<TagEncoding>,
}

impl EncodingOptions {
    pub fn key_to_tag(&self, key: &str) -> (String, Vec<String>) {
        let main = self.tag_encoding.key_to_tag(key);
        let fallbacks = self
            .fallback_encodings
            .iter()
            .map(|e| e.key_to_tag(key))
            .collect();
        (main, fallbacks)
    }
    pub fn tag_to_key(&self, tag: &str) -> Result<String, Error> {
        let mut errors = vec![];
        let main = [self.tag_encoding];
        let encodings = main.iter().chain(self.fallback_encodings.iter());
        for e in encodings {
            match e.tag_to_key(tag) {
                Ok(r) => return Ok(r),
                Err(e) => errors.push(e),
            }
        }
        Err(Error::TagToKey(errors))
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum TagEncoding {
    // A custom encoding
    Custom,
    // https://docs.rs/data-encoding/latest/data_encoding/constant.BASE32_DNSSEC.html
    // It uses a base32 extended hex alphabet.
    // It is case-insensitive when decoding and uses lowercase when encoding.
    // It does not use padding.
    Base32DNSSEC,
}

static CUSTOM_ENCODING: Lazy<CustomEncoding> = Lazy::new(CustomEncoding::new);

impl TagEncoding {
    pub fn key_to_tag(&self, key: &str) -> String {
        match self {
            TagEncoding::Custom => CUSTOM_ENCODING.encode(key),
            TagEncoding::Base32DNSSEC => BASE32_DNSSEC.encode(key.as_bytes()),
        }
    }

    pub fn tag_to_key(&self, tag: &str) -> Result<String, Error> {
        match self {
            TagEncoding::Custom => CUSTOM_ENCODING.decode(tag),
            TagEncoding::Base32DNSSEC => {
                Ok(String::from_utf8(BASE32_DNSSEC.decode(tag.as_bytes())?)?)
            }
        }
    }
}

/// A tag MUST be at most 128 characters in length and MUST match the following regular expression:
/// [a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}
/// https://github.com/opencontainers/distribution-spec/blob/main/spec.md
#[derive(Clone, Debug)]
pub struct CustomEncoding {
    symbol_table: Vec<char>,
    reverse_table: BTreeMap<char, u32>,
}

impl Default for CustomEncoding {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomEncoding {
    pub fn new() -> CustomEncoding {
        let mut symbol_table = Vec::new();
        symbol_table.extend('0'..='9');
        symbol_table.extend('a'..='z');
        symbol_table.extend('A'..='Z');
        symbol_table.push('-');
        symbol_table.push('.');

        let mut reverse_table = BTreeMap::new();

        for (i, c) in symbol_table.iter().enumerate() {
            reverse_table.insert(*c, i as u32);
        }

        CustomEncoding {
            symbol_table,
            reverse_table,
        }
    }

    pub fn encode(&self, key: &str) -> String {
        let mut result = String::new();
        let mut first = true;
        for c in key.chars() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' => result.push(c),
                '-' | '.' => {
                    if first {
                        self.encode_char(&mut result, c);
                    } else {
                        result.push(c);
                    }
                }
                _ => self.encode_char(&mut result, c),
            }
            first = false;
        }
        result
    }

    fn encode_char(&self, result: &mut String, c: char) {
        result.push('_');

        let mut n: u32 = c.into();

        let mut char_code = Vec::new();
        let base = self.symbol_table.len() as u32;
        while n != 0 {
            let quotient = n / base;
            let remainder = n % base;

            char_code.push(self.symbol_table[remainder as usize]);

            n = quotient;
        }
        result.extend(char_code.iter().rev());

        result.push('_');
    }

    pub fn decode(&self, tag: &str) -> Result<String, Error> {
        let mut chars = tag.chars();
        let mut result = String::new();
        while let Some(c) = chars.next() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' => result.push(c),
                '_' => {
                    let mut encoded_char = Vec::new();
                    loop {
                        match chars.next() {
                            Some('_') => break,
                            Some(n) => encoded_char.push(n),
                            None => return Err(Error::InvalidTag(tag.to_string())),
                        }
                    }
                    result.push(
                        self.decode_char(&encoded_char)
                            .ok_or_else(|| Error::InvalidTag(tag.to_string()))?,
                    )
                }
                _ => return Err(Error::InvalidTag(tag.to_string())),
            }
        }
        Ok(result)
    }

    fn decode_char(&self, encoded: &[char]) -> Option<char> {
        let base = self.symbol_table.len() as u32;
        let mut n = 0u32;
        for c in encoded.iter() {
            n = n.checked_mul(base)?;
            n = n.checked_add(*self.reverse_table.get(c)?)?;
        }
        n.try_into().ok()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn custom_encode_symbol_table_length() {
        assert_eq!(CUSTOM_ENCODING.symbol_table.len(), 64);
    }

    #[test]
    fn custom_encode_symbol_table_validate() {
        assert_eq!(
            CUSTOM_ENCODING.symbol_table.len(),
            CUSTOM_ENCODING.reverse_table.len()
        );
        for (i, c) in CUSTOM_ENCODING.symbol_table.iter().enumerate() {
            assert_eq!(CUSTOM_ENCODING.reverse_table[c], i as u32);
        }
        for (c, i) in CUSTOM_ENCODING.reverse_table.iter() {
            assert_eq!(CUSTOM_ENCODING.symbol_table[*i as usize], *c);
        }
    }

    #[test]
    fn custom_encode_id() {
        assert_eq!(
            CUSTOM_ENCODING
                .encode("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-."),
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-."
        );
    }

    #[test]
    fn custom_encode_first_special() {
        assert_eq!(CUSTOM_ENCODING.encode("--"), "_J_-");
        assert_eq!(CUSTOM_ENCODING.encode(".."), "_K_.");
        assert_eq!(CUSTOM_ENCODING.encode("//"), "_L__L_");
        assert_eq!(CUSTOM_ENCODING.encode("__"), "_1v__1v_");
    }

    #[test]
    fn custom_decode_id() {
        assert_eq!(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-.",
            CUSTOM_ENCODING
                .decode("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-.")
                .unwrap()
        );
    }

    #[test]
    fn custom_decode_first_special() {
        assert_eq!(("--"), CUSTOM_ENCODING.decode("_J_-").unwrap());
        assert_eq!((".."), CUSTOM_ENCODING.decode("_K_.").unwrap());
        assert_eq!(("//"), CUSTOM_ENCODING.decode("_L__L_").unwrap());
        assert_eq!(("__"), CUSTOM_ENCODING.decode("_1v__1v_").unwrap());
    }

    #[test]
    fn custom_encode_decode() {
        let test_strings = [
            "test",
            "测试",
            "_test-测试_",
            "._test-测试_.",
            "._test-测试_.测试",
            "realisations/sha256:67890e0958e5d1a2944a3389151472a9acde025c7812f68381a7eef0d82152d1!libgcc.doi",
        ];
        for s in test_strings {
            let encoded = CUSTOM_ENCODING.encode(s);
            // https://github.com/opencontainers/distribution-spec/blob/main/spec.md#pulling-manifests
            assert!(encoded.len() <= 128);
            assert_eq!(CUSTOM_ENCODING.decode(&encoded).unwrap(), s);
        }
    }
}
