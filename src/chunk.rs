//! Bounded in-memory channel passing data written by blocking code to
//! asynchronous code as a stream of chunks.

use std::io::{self, Write};

use bytes::{Bytes, BytesMut};
use futures::{Stream, stream};
use tokio::sync::mpsc;

/// Creates a channel whose writer sends data in chunks of `chunk_size`
/// bytes, blocking while `capacity` chunks are waiting to be received.
///
/// At most `capacity + 1` chunks are buffered by the channel. The stream
/// ends once the writer finishes, and fails if the writer is dropped
/// without finishing.
pub fn channel(
    chunk_size: usize,
    capacity: usize,
) -> (
    ChunkWriter,
    impl Stream<Item = Result<Bytes, WriterDropped>> + Send + 'static,
) {
    assert!(chunk_size > 0, "chunk size must be positive");
    let (sender, receiver) = mpsc::channel(capacity);
    let writer = ChunkWriter {
        buffer: BytesMut::new(),
        chunk_size,
        sender,
    };
    let chunks = stream::unfold(Some(receiver), |receiver| async move {
        let mut receiver = receiver?;
        match receiver.recv().await {
            Some(Message::Chunk(chunk)) => Some((Ok(chunk), Some(receiver))),
            Some(Message::End) => None,
            None => Some((Err(WriterDropped), None)),
        }
    });
    (writer, chunks)
}

/// Writer half of [`channel`].
///
/// Writes block while the channel is full, so it must not be used in
/// asynchronous contexts.
pub struct ChunkWriter {
    buffer: BytesMut,
    chunk_size: usize,
    sender: mpsc::Sender<Message>,
}

enum Message {
    Chunk(Bytes),
    /// The writer finished, no chunk follows.
    End,
}

/// Error of reading from a chunk stream whose [`ChunkWriter`] was dropped
/// before finishing.
#[derive(thiserror::Error, Debug)]
#[error("chunk writer dropped before finishing")]
pub struct WriterDropped;

/// Error of writing to a [`ChunkWriter`] whose receiving stream is dropped.
#[derive(thiserror::Error, Debug)]
#[error("chunk stream dropped")]
pub struct StreamDropped;

/// Returns whether `error` comes from writing to a [`ChunkWriter`] whose
/// receiving stream is dropped.
pub fn is_stream_dropped(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|e| e.is::<StreamDropped>())
}

impl ChunkWriter {
    /// Sends the remaining buffered data and ends the stream.
    pub fn finish(mut self) -> io::Result<()> {
        self.send_buffer()?;
        self.send(Message::End)
    }

    fn send_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = self.buffer.split().freeze();
        self.send(Message::Chunk(chunk))
    }

    fn send(&self, message: Message) -> io::Result<()> {
        self.sender
            .blocking_send(message)
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
