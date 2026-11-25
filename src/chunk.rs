//! Bounded in-memory channel passing data written by blocking code to
//! asynchronous code as a stream of chunks.

use std::io::{self, Write};

use bytes::{Bytes, BytesMut};
use futures::{Stream, stream};
use tokio::sync::mpsc;

/// Creates a channel whose writer sends data in chunks of `chunk_size`
/// bytes, blocking while `capacity` chunks are waiting to be received.
///
/// At most `capacity + 1` chunks are buffered by the channel.
pub fn channel(
    chunk_size: usize,
    capacity: usize,
) -> (ChunkWriter, impl Stream<Item = Bytes> + Send + 'static) {
    assert!(chunk_size > 0, "chunk size must be positive");
    let (sender, mut receiver) = mpsc::channel(capacity);
    let writer = ChunkWriter {
        buffer: BytesMut::new(),
        chunk_size,
        sender,
    };
    let chunks = stream::poll_fn(move |cx| receiver.poll_recv(cx));
    (writer, chunks)
}

/// Writer half of [`channel`].
///
/// Writes block while the channel is full, so it must not be used in
/// asynchronous contexts.
pub struct ChunkWriter {
    buffer: BytesMut,
    chunk_size: usize,
    sender: mpsc::Sender<Bytes>,
}

/// Error of writing to a [`ChunkWriter`] whose receiving stream is dropped.
#[derive(thiserror::Error, Debug)]
#[error("chunk stream dropped")]
pub struct StreamDropped;

impl ChunkWriter {
    /// Sends the remaining buffered data.
    pub fn finish(mut self) -> io::Result<()> {
        self.send_buffer()
    }

    fn send_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = self.buffer.split().freeze();
        self.sender
            .blocking_send(chunk)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, StreamDropped))
    }
}

impl Write for ChunkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.buffer.len() == self.chunk_size {
            self.send_buffer()?;
        }
        if self.buffer.is_empty() {
            self.buffer.reserve(self.chunk_size);
        }
        let len = buf.len().min(self.chunk_size - self.buffer.len());
        self.buffer.extend_from_slice(&buf[..len]);
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buffer()
    }
}
