use data_encoding::BASE32_DNSSEC;

use crate::error::Error;

pub fn key_to_tag(key: &str) -> String {
    // https://docs.rs/data-encoding/latest/data_encoding/constant.BASE32_DNSSEC.html
    // It uses a base32 extended hex alphabet.
    // It is case-insensitive when decoding and uses lowercase when encoding.
    // It does not use padding.
    BASE32_DNSSEC.encode(key.as_bytes())
}

pub fn tag_to_key(tag: &str) -> Result<String, Error> {
    Ok(String::from_utf8(BASE32_DNSSEC.decode(tag.as_bytes())?)?)
}
