use std::io::{self, Write};

use sha2::{Digest, Sha256};

/// Writer adapter that computes the SHA-256 hash and the size of all
/// data successfully written to the inner writer.
pub struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    size: u64,
}

impl<W> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            size: 0,
        }
    }

    /// Returns the inner writer, the number of bytes written and their
    /// SHA-256 hash.
    pub fn finish(self) -> (W, u64, [u8; 32]) {
        (self.inner, self.size, self.hasher.finalize().into())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
