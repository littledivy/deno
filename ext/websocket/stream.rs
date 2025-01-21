// Copyright 2018-2025 the Deno authors. MIT license.
use std::io::ErrorKind;
use std::ops::Deref;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::ready;
use std::task::Poll;

use bytes::Buf;
use bytes::Bytes;
use deno_net::raw::NetworkStream;
use h2::RecvStream;
use h2::SendStream;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

// TODO(bartlomieju): remove this
pub(crate) enum WsStreamKind {
  Upgraded(TokioIo<Upgraded>),
  Network(NetworkStream),
  H2(SendStream<Bytes>, RecvStream),
}

pub(crate) struct WebSocketStreamWrite {
  write_half: Arc<Mutex<WsStreamKind>>,
}

impl WebSocketStreamWrite {
  fn with_write_lock<R>(&self, f: impl FnOnce(&mut WsStreamKind) -> R) -> R {
    let mut write_half = self.write_half.lock().unwrap();
    f(&mut write_half)
  }
}

pub(crate) struct WebSocketStream {
  read_half: Arc<Mutex<WsStreamKind>>,
  write_half: Option<WebSocketStreamWrite>,
  pre: Option<Bytes>,
}

impl WebSocketStream {
  pub fn new(stream: WsStreamKind, buffer: Option<Bytes>) -> Self {
    let inner = Arc::new(Mutex::new(stream));
    Self {
      read_half: inner.clone(),
      write_half: Some(WebSocketStreamWrite { write_half: inner }),
      pre: buffer,
    }
  }

  fn with_read_lock<R>(&self, f: impl FnOnce(&mut WsStreamKind) -> R) -> R {
    let mut read_half = self.read_half.lock().unwrap();
    f(&mut read_half)
  }
}

pub(crate) fn split(
  mut ws: WebSocketStream,
) -> (WebSocketStream, WebSocketStreamWrite) {
  let wr = ws.write_half.take().expect("already split");
  (ws, wr)
}

impl Deref for WebSocketStream {
  type Target = WebSocketStreamWrite;

  fn deref(&self) -> &Self::Target {
    self.write_half.as_ref().unwrap()
  }
}

impl DerefMut for WebSocketStream {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.write_half.as_mut().unwrap()
  }
}

impl AsyncRead for WebSocketStream {
  // From hyper's Rewind (https://github.com/hyperium/hyper), MIT License, Copyright (c) Sean McArthur
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<std::io::Result<()>> {
    if let Some(mut prefix) = self.pre.take() {
      // If there are no remaining bytes, let the bytes get dropped.
      if !prefix.is_empty() {
        let copy_len = std::cmp::min(prefix.len(), buf.remaining());
        // TODO: There should be a way to do following two lines cleaner...
        buf.put_slice(&prefix[..copy_len]);
        prefix.advance(copy_len);
        // Put back what's left
        if !prefix.is_empty() {
          self.pre = Some(prefix);
        }

        return Poll::Ready(Ok(()));
      }
    }
    self.with_read_lock(|stream| match stream {
      WsStreamKind::Network(stream) => Pin::new(stream).poll_read(cx, buf),
      WsStreamKind::Upgraded(stream) => Pin::new(stream).poll_read(cx, buf),
      WsStreamKind::H2(_, recv) => {
        let data = ready!(recv.poll_data(cx));
        let Some(data) = data else {
          // EOF
          return Poll::Ready(Ok(()));
        };
        let mut data = data.map_err(|e| {
          std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        recv.flow_control().release_capacity(data.len()).unwrap();
        // This looks like the prefix code above -- can we share this?
        let copy_len = std::cmp::min(data.len(), buf.remaining());
        // TODO: There should be a way to do following two lines cleaner...
        buf.put_slice(&data[..copy_len]);
        data.advance(copy_len);
        // Put back what's left
        if !data.is_empty() {
          //self.pre = Some(data);
        }
        Poll::Ready(Ok(()))
      }
    })
  }
}

impl AsyncWrite for WebSocketStream {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    buf: &[u8],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    let stream = Pin::new(&mut *self.get_mut().write_half.as_mut().unwrap());
    stream.poll_write(cx, buf)
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    let stream = Pin::new(&mut *self.get_mut().write_half.as_mut().unwrap());
    stream.poll_flush(cx)
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    let stream = Pin::new(&mut *self.get_mut().write_half.as_mut().unwrap());
    stream.poll_shutdown(cx)
  }

  fn is_write_vectored(&self) -> bool {
    self.write_half.as_ref().unwrap().is_write_vectored()
  }

  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    let stream = Pin::new(&mut *self.get_mut().write_half.as_mut().unwrap());
    stream.poll_write_vectored(cx, bufs)
  }
}

impl AsyncWrite for WebSocketStreamWrite {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    buf: &[u8],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    self.with_write_lock(|stream| match stream {
      WsStreamKind::Network(stream) => Pin::new(stream).poll_write(cx, buf),
      WsStreamKind::Upgraded(stream) => Pin::new(stream).poll_write(cx, buf),
      WsStreamKind::H2(send, _) => {
        // Zero-length write succeeds
        if buf.is_empty() {
          return Poll::Ready(Ok(0));
        }

        send.reserve_capacity(buf.len());
        let res = ready!(send.poll_capacity(cx));

        // TODO(mmastrac): the documentation is not entirely clear what to do here, so we'll continue
        _ = res;

        // We'll try to send whatever we have capacity for
        let size = std::cmp::min(buf.len(), send.capacity());
        assert!(size > 0);

        let buf: Bytes = Bytes::copy_from_slice(&buf[0..size]);
        let len = buf.len();
        // TODO(mmastrac): surface the h2 error?
        let res = send
          .send_data(buf, false)
          .map_err(|_| std::io::Error::from(ErrorKind::Other));
        Poll::Ready(res.map(|_| len))
      }
    })
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    self.with_write_lock(|stream| match stream {
      WsStreamKind::Network(stream) => Pin::new(stream).poll_flush(cx),
      WsStreamKind::Upgraded(stream) => Pin::new(stream).poll_flush(cx),
      WsStreamKind::H2(..) => Poll::Ready(Ok(())),
    })
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    self.with_write_lock(|stream| match stream {
      WsStreamKind::Network(stream) => Pin::new(stream).poll_shutdown(cx),
      WsStreamKind::Upgraded(stream) => Pin::new(stream).poll_shutdown(cx),
      WsStreamKind::H2(send, _) => {
        // TODO(mmastrac): surface the h2 error?
        let res = send
          .send_data(Bytes::new(), false)
          .map_err(|_| std::io::Error::from(ErrorKind::Other));
        Poll::Ready(res)
      }
    })
  }

  fn is_write_vectored(&self) -> bool {
    self.with_write_lock(|stream| match stream {
      WsStreamKind::Network(stream) => stream.is_write_vectored(),
      WsStreamKind::Upgraded(stream) => stream.is_write_vectored(),
      WsStreamKind::H2(..) => false,
    })
  }

  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    self.with_write_lock(|stream| match stream {
      WsStreamKind::Network(stream) => {
        Pin::new(stream).poll_write_vectored(cx, bufs)
      }
      WsStreamKind::Upgraded(stream) => {
        Pin::new(stream).poll_write_vectored(cx, bufs)
      }
      WsStreamKind::H2(..) => {
        // TODO(mmastrac): this is possibly just too difficult, but we'll never call it
        unimplemented!()
      }
    })
  }
}
