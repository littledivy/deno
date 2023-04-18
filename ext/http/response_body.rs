// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.
use std::borrow::Cow;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Waker;

use deno_core::error::bad_resource;
use deno_core::error::AnyError;
use deno_core::futures::FutureExt;
use deno_core::AsyncRefCell;
use deno_core::AsyncResult;
use deno_core::BufView;
use deno_core::CancelHandle;
use deno_core::CancelTryFuture;
use deno_core::RcRef;
use deno_core::Resource;
use deno_core::WriteOutcome;
use hyper1::body::Body;
use hyper1::body::Frame;
use hyper1::body::SizeHint;

#[derive(Clone, Debug, Default)]
pub struct CompletionHandle {
  inner: Rc<RefCell<CompletionHandleInner>>,
}

#[derive(Debug, Default)]
struct CompletionHandleInner {
  complete: bool,
  success: bool,
  waker: Option<Waker>,
}

impl CompletionHandle {
  pub fn complete(&self, success: bool) {
    let mut mut_self = self.inner.borrow_mut();
    mut_self.complete = true;
    mut_self.success = success;
    if let Some(waker) = mut_self.waker.take() {
      drop(mut_self);
      waker.wake();
    }
  }
}

impl Future for CompletionHandle {
  type Output = bool;

  fn poll(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Self::Output> {
    let mut mut_self = self.inner.borrow_mut();
    if mut_self.complete {
      return std::task::Poll::Ready(mut_self.success);
    }

    mut_self.waker = Some(cx.waker().clone());
    std::task::Poll::Pending
  }
}

#[derive(Default)]
pub enum ResponseBytesInner {
  /// An empty stream.
  #[default]
  Empty,
  /// A completed stream.
  Done,
  /// A static buffer of bytes, sent it one fell swoop.
  Bytes(BufView),
  /// A resource stream, piped in fast mode.
  Resource(bool, Rc<dyn Resource>, AsyncResult<BufView>),
  /// A JS-backed stream, written in JS and transported via pipe.
  V8Stream(tokio::sync::mpsc::Receiver<BufView>),
}

impl std::fmt::Debug for ResponseBytesInner {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Done => f.write_str("Done"),
      Self::Empty => f.write_str("Empty"),
      Self::Bytes(..) => f.write_str("Bytes"),
      Self::Resource(..) => f.write_str("Resource"),
      Self::V8Stream(..) => f.write_str("V8Stream"),
    }
  }
}

/// This represents the union of possible response types in Deno with the stream-style [`Body`] interface
/// required by hyper. As the API requires information about request completion (including a success/fail
/// flag), we include a very lightweight [`CompletionHandle`] for interested parties to listen on.
#[derive(Debug, Default)]
pub struct ResponseBytes(ResponseBytesInner, CompletionHandle);

impl ResponseBytes {
  pub fn initialize(&mut self, inner: ResponseBytesInner) {
    debug_assert!(matches!(self.0, ResponseBytesInner::Empty));
    self.0 = inner;
  }

  pub fn completion_handle(&self) -> CompletionHandle {
    self.1.clone()
  }

  fn complete(&mut self, success: bool) -> ResponseBytesInner {
    if matches!(self.0, ResponseBytesInner::Done) {
      return ResponseBytesInner::Done;
    }

    let current = std::mem::replace(&mut self.0, ResponseBytesInner::Done);
    self.1.complete(success);
    current
  }
}

impl ResponseBytesInner {
  pub fn len(&self) -> Option<usize> {
    match self {
      Self::Done => Some(0),
      Self::Empty => Some(0),
      Self::Bytes(bytes) => Some(bytes.len()),
      Self::Resource(..) => None,
      Self::V8Stream(..) => None,
    }
  }
}

impl Body for ResponseBytes {
  type Data = BufView;
  type Error = AnyError;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
    match &mut self.0 {
      ResponseBytesInner::Done | ResponseBytesInner::Empty => {
        unreachable!()
      }
      ResponseBytesInner::Bytes(..) => {
        if let ResponseBytesInner::Bytes(data) = self.complete(true) {
          std::task::Poll::Ready(Some(Ok(Frame::data(data))))
        } else {
          unreachable!()
        }
      }
      ResponseBytesInner::Resource(auto_close, stm, ref mut future) => {
        match future.poll_unpin(cx) {
          std::task::Poll::Pending => std::task::Poll::Pending,
          std::task::Poll::Ready(Err(err)) => {
            std::task::Poll::Ready(Some(Err(err)))
          }
          std::task::Poll::Ready(Ok(buf)) => {
            if buf.is_empty() {
              if *auto_close {
                stm.clone().close();
              }
              self.complete(true);
              return std::task::Poll::Ready(None);
            }
            // Re-arm the future
            *future = stm.clone().read(64 * 1024);
            std::task::Poll::Ready(Some(Ok(Frame::data(buf))))
          }
        }
      }
      ResponseBytesInner::V8Stream(stm) => match stm.poll_recv(cx) {
        std::task::Poll::Pending => std::task::Poll::Pending,
        std::task::Poll::Ready(Some(buf)) => {
          std::task::Poll::Ready(Some(Ok(Frame::data(buf))))
        }
        std::task::Poll::Ready(None) => {
          self.complete(true);
          std::task::Poll::Ready(None)
        }
      },
    }
  }

  fn is_end_stream(&self) -> bool {
    matches!(self.0, ResponseBytesInner::Done | ResponseBytesInner::Empty)
  }

  fn size_hint(&self) -> SizeHint {
    if let Some(size) = self.0.len() {
      SizeHint::with_exact(size as u64)
    } else {
      SizeHint::default()
    }
  }
}

impl Drop for ResponseBytes {
  fn drop(&mut self) {
    // We won't actually poll_frame for Empty responses so this is where we return success
    self.complete(matches!(self.0, ResponseBytesInner::Empty));
  }
}

/// A response body object that can be passed to V8. This body will feed byte buffers to a channel which
/// feed's hyper's HTTP response.
pub struct V8StreamHttpResponseBody(
  AsyncRefCell<Option<tokio::sync::mpsc::Sender<BufView>>>,
  CancelHandle,
);

impl V8StreamHttpResponseBody {
  pub fn new(sender: tokio::sync::mpsc::Sender<BufView>) -> Self {
    Self(AsyncRefCell::new(Some(sender)), CancelHandle::default())
  }
}

impl Resource for V8StreamHttpResponseBody {
  fn name(&self) -> Cow<str> {
    "responseBody".into()
  }

  fn write(
    self: Rc<Self>,
    buf: BufView,
  ) -> AsyncResult<deno_core::WriteOutcome> {
    let cancel_handle = RcRef::map(&self, |this| &this.1);
    Box::pin(
      async move {
        let nwritten = buf.len();

        let res = RcRef::map(self, |this| &this.0).borrow().await;
        if let Some(tx) = res.as_ref() {
          tx.send(buf)
            .await
            .map_err(|_| bad_resource("failed to write"))?;
          Ok(WriteOutcome::Full { nwritten })
        } else {
          Err(bad_resource("failed to write"))
        }
      }
      .try_or_cancel(cancel_handle),
    )
  }

  fn close(self: Rc<Self>) {
    self.1.cancel();
  }
}
