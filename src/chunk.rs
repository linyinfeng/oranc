//! Streaming chunk adapters: [`channel`] transfers data written by
//! blocking code into an asynchronous chunk stream, and [`rechunk`]
//! normalizes an existing byte stream into fixed-size chunks.

use std::io::{self, Write};
use std::pin::Pin;

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt, stream};
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
            .map_err(|_| stream_dropped())
    }
}

fn stream_dropped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, StreamDropped)
}

impl Write for ChunkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.sender.is_closed() {
            return Err(stream_dropped());
        }
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
        if self.sender.is_closed() {
            return Err(stream_dropped());
        }
        self.send_buffer()
    }
}

/// Repackages the byte stream `stream` into chunks of `chunk_size` bytes
/// (plus a smaller final chunk if the total length is not a multiple of
/// `chunk_size`), never emitting empty chunks.
///
/// Errors from `stream` are forwarded immediately and terminate the stream.
pub fn rechunk<S, E>(
    stream: S,
    chunk_size: usize,
) -> impl Stream<Item = Result<Bytes, E>> + Send + 'static
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    assert!(chunk_size > 0, "chunk size must be positive");
    let state = RechunkState {
        stream: Box::pin(stream),
        buffer: BytesMut::new(),
        pending: Bytes::new(),
    };
    stream::unfold(Some(state), move |state| next_chunk(state, chunk_size))
}

struct RechunkState<S> {
    stream: Pin<Box<S>>,
    buffer: BytesMut,
    pending: Bytes,
}

async fn next_chunk<S, E>(
    state: Option<RechunkState<S>>,
    chunk_size: usize,
) -> Option<(Result<Bytes, E>, Option<RechunkState<S>>)>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    let mut state = state?;
    loop {
        if !state.pending.is_empty() {
            // Avoid copying when an incoming frame already fills a full chunk.
            if state.buffer.is_empty() && state.pending.len() >= chunk_size {
                let chunk = state.pending.split_to(chunk_size);
                return Some((Ok(chunk), Some(state)));
            }
            let len = state.pending.len().min(chunk_size - state.buffer.len());
            state.buffer.extend_from_slice(&state.pending.split_to(len));
            if state.buffer.len() == chunk_size {
                let chunk = std::mem::take(&mut state.buffer).freeze();
                return Some((Ok(chunk), Some(state)));
            }
        }
        match state.stream.as_mut().next().await {
            Some(Ok(data)) => state.pending = data,
            Some(Err(err)) => return Some((Err(err), None)),
            None => {
                return (!state.buffer.is_empty()).then(|| (Ok(state.buffer.freeze()), None));
            }
        }
    }
}
