use deno_core::error::AnyError;
use deno_core::futures::task::Context;
use deno_core::op;
use deno_core::OpState;
use deno_core::ResourceId;
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

use crate::Request;

// Wrapper type for tokio::net::TcpStream that implements
// deno_websocket::UpgradedStream
struct UpgradedStream(tokio::net::TcpStream);
impl tokio::io::AsyncRead for UpgradedStream {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context,
    buf: &mut tokio::io::ReadBuf,
  ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
    Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
  }
}

impl tokio::io::AsyncWrite for UpgradedStream {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context,
    buf: &[u8],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
  }
  fn poll_flush(
    self: Pin<&mut Self>,
    cx: &mut Context,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    Pin::new(&mut self.get_mut().0).poll_flush(cx)
  }
  fn poll_shutdown(
    self: Pin<&mut Self>,
    cx: &mut Context,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
  }
}

impl deno_websocket::Upgraded for UpgradedStream {}

#[op]
pub async fn op_flash_upgrade_websocket(
  state: Rc<RefCell<OpState>>,
  rid: u32,
) -> Result<ResourceId, AnyError> {
  let stream = state.borrow_mut().resource_table.take::<Request>(rid)?;
  let stream = Rc::try_unwrap(stream.inner.inner.clone())
    .unwrap()
    .into_inner();
  deno_websocket::ws_create_server_stream(
    &state,
    Box::pin(UpgradedStream(stream)),
  )
  .await
}
