//! A stream of byte chunks as an [`AsyncRead`]: how a remote's frames
//! (vox chunks, an HTTP body, a growing local file) become the
//! [`Reader`](super::Reader) a [`Source`](super::Source) hands back.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::io::{AsyncRead, ReadBuf};

/// Reads the chunks of `S` in order. An `Err` item is returned from the
/// read that reaches it; the stream ending is the end of the bytes.
pub struct StreamReader<S> {
    stream: S,
    chunk: Vec<u8>,
    at: usize,
}

impl<S> StreamReader<S> {
    pub const fn new(stream: S) -> Self {
        Self {
            stream,
            chunk: Vec::new(),
            at: 0,
        }
    }
}

impl<S> AsyncRead for StreamReader<S>
where
    S: Stream<Item = io::Result<Vec<u8>>> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        while this.at >= this.chunk.len() {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(Some(Ok(chunk))) => {
                    this.chunk = chunk;
                    this.at = 0;
                }
            }
        }
        let rest = this.chunk.get(this.at..).unwrap_or_default();
        let n = rest.len().min(buf.remaining());
        buf.put_slice(rest.get(..n).unwrap_or_default());
        this.at = this.at.saturating_add(n);
        Poll::Ready(Ok(()))
    }
}
