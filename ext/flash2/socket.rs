use async_http_codec::BodyDecode;
use std::io;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

pub enum IOSocket {
  Tcp(TcpStream),
}

impl AsyncRead for IOSocket {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    match self.get_mut() {
      IOSocket::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
    }
  }
}

impl AsyncWrite for IOSocket {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<io::Result<usize>> {
    match self.get_mut() {
      IOSocket::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
    }
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<io::Result<()>> {
    match self.get_mut() {
      IOSocket::Tcp(stream) => Pin::new(stream).poll_flush(cx),
    }
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<io::Result<()>> {
    match self.get_mut() {
      IOSocket::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
    }
  }
}

// pub struct Socket {
//   pub inner: Option<IOSocket>,
//   pub body_decode: Option<BodyDecode<IOSocket>>,
// }
