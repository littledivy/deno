// Copyright 2018-2026 the Deno authors. MIT license.

use std::cell::RefCell;
use std::rc::Rc;

use deno_core::BufView;
use deno_core::JsBuffer;
use deno_core::OpState;
use deno_core::ResourceId;
use deno_core::error::ResourceError;
use deno_core::op2;
use deno_error::JsErrorBox;
use deno_net::io::TcpStreamResource;
#[cfg(unix)]
use deno_net::io::UnixStreamResource;

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum StreamWriteError {
  #[class(inherit)]
  #[error(transparent)]
  Resource(#[from] ResourceError),
  #[class(inherit)]
  #[error(transparent)]
  Io(#[from] JsErrorBox),
}

/// Attempt a synchronous (non-blocking) write on a stream resource.
/// Returns bytes written (>= 0) on success, or -1 if the write would
/// block or the resource type doesn't support try_write.
#[op2(fast)]
#[smi]
pub fn op_node_try_write(
  state: &mut OpState,
  #[smi] rid: ResourceId,
  #[buffer] buf: &[u8],
) -> i32 {
  // Try TCP stream first
  if let Ok(resource) = state.resource_table.get::<TcpStreamResource>(rid) {
    match resource.try_write_sync(buf) {
      Ok(nwritten) => return nwritten as i32,
      Err(_) => return -1,
    }
  }

  // Try Unix stream
  #[cfg(unix)]
  if let Ok(resource) = state.resource_table.get::<UnixStreamResource>(rid) {
    match resource.try_write_sync(buf) {
      Ok(nwritten) => return nwritten as i32,
      Err(_) => return -1,
    }
  }

  // Unsupported resource type
  -1
}

/// Async write-all on a stream resource. Used as fallback when try_write
/// can't complete the entire write synchronously.
#[op2]
#[number]
pub async fn op_node_stream_write(
  state: Rc<RefCell<OpState>>,
  #[smi] rid: ResourceId,
  #[buffer] buf: JsBuffer,
) -> Result<usize, StreamWriteError> {
  let resource = state.borrow().resource_table.get_any(rid)?;
  let len = buf.len();
  resource.write_all(BufView::from(buf)).await?;
  Ok(len)
}
