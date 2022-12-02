// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

use deno_core::error::generic_error;
use deno_core::error::type_error;
use deno_core::error::AnyError;
use deno_core::op;
use deno_core::serde_v8;
use deno_core::v8;
use deno_core::ByteString;
use deno_core::CancelFuture;
use deno_core::CancelHandle;
use deno_core::Extension;
use deno_core::OpState;
use deno_core::StringOrBuffer;
use deno_core::ZeroCopyBuf;
use std::borrow::Cow;
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{
  unbounded_channel, UnboundedReceiver, UnboundedSender,
};

pub struct Unstable(pub bool);

fn check_unstable(state: &OpState, api_name: &str) {
  let unstable = state.borrow::<Unstable>();

  if !unstable.0 {
    eprintln!(
      "Unstable API '{}'. The --unstable flag must be provided.",
      api_name
    );
    std::process::exit(70);
  }
}

pub trait FlashPermissions {
  fn check_net<T: AsRef<str>>(
    &mut self,
    _host: &(T, Option<u16>),
    _api_name: &str,
  ) -> Result<(), AnyError>;
}

struct Channel {
  inner: Rc<RefCell<UnboundedReceiver<Request>>>,
}

unsafe impl Send for Channel {}
unsafe impl Sync for Channel {}

impl deno_core::Resource for Channel {
  fn name(&self) -> Cow<str> {
    "httpChannel".into()
  }
}
#[derive(Debug, Clone)]
struct Request {
  inner: Rc<RefCell<tokio::net::TcpStream>>,
}
impl deno_core::Resource for Request {
  fn name(&self) -> Cow<str> {
    "httpRequest".into()
  }
}

unsafe impl Send for Request {}
unsafe impl Sync for Request {}

#[derive(Clone, Copy)]
struct JsCb {
  isolate: *mut v8::Isolate,
  js_cb: *mut v8::Function,
  context: *mut v8::Context,
}

impl JsCb {
  fn call(&self, rid: u32) {
    let js_cb = unsafe { &mut *self.js_cb };
    let isolate = unsafe { &mut *self.isolate };
    let context = unsafe {
      std::mem::transmute::<*mut v8::Context, v8::Local<v8::Context>>(
        self.context,
      )
    };
    let recv = v8::undefined(isolate).into();
    let scope = &mut v8::HandleScope::with_context(isolate, context);
    let args = &[v8::Integer::new(scope, rid as i32).into()];
    let _ = js_cb.call(scope, recv, args);
  }
}

unsafe impl Send for JsCb {}
unsafe impl Sync for JsCb {}

#[derive(Clone, Copy)]
struct SharedOpState(*mut OpState);
unsafe impl Send for SharedOpState {}
unsafe impl Sync for SharedOpState {}

impl SharedOpState {
  fn add_resource(&self, r: Request) -> u32 {
    let state = unsafe { &mut *self.0 };
    state.resource_table.add(r)
  }
}

#[op(v8)]
fn op_flash_start(
  scope: &mut v8::HandleScope,
  state: &mut OpState,
  js_cb: serde_v8::Value,
) -> Result<impl Future<Output = Result<(), AnyError>>, AnyError> {
  let current_context = scope.get_current_context();
  let context = v8::Global::new(scope, current_context).into_raw();
  let isolate: *mut v8::Isolate = &mut *scope as &mut v8::Isolate;
  let js_cb = JsCb {
    isolate,
    js_cb: v8::Global::new(scope, js_cb.v8_value).into_raw().as_ptr()
      as *mut v8::Function,
    context: context.as_ptr(),
  };

  let state = SharedOpState(state as *mut OpState);

  Ok(async move {
    let listener = TcpListener::bind("127.0.0.1:4500").await.unwrap();
    loop {
      let (socket, _) = listener.accept().await.unwrap();
      let socket = Request {
        inner: Rc::new(RefCell::new(socket)),
      };

      let server_socket = unsafe { &mut *socket.inner.as_ptr() };

      tokio::task::spawn(async move {
        let mut read_buf = vec![0_u8; 1024];

        loop {
          let mut headers = [httparse::EMPTY_HEADER; 40];
          let mut req = httparse::Request::new(&mut headers);
          let nread = server_socket.read(&mut read_buf).await;
          match nread {
            Ok(0) => {
              break;
            }
            Ok(n) => {
              let _ = req.parse(&read_buf[..n]);

              js_cb.call(state.add_resource(socket.clone()));
            }
            _ => {}
          }
        }
      });
    }
    Ok(())
  })
}

#[op]
fn op_flash_try_write(
  state: &mut OpState,
  rid: u32,
  buffer: &[u8],
) -> Result<u32, AnyError> {
  let req = state.resource_table.take::<Request>(rid)?;
  let sock = req.inner.borrow_mut();
  Ok(sock.try_write(buffer)? as u32)
}

pub fn init<P: FlashPermissions + 'static>(unstable: bool) -> Extension {
  Extension::builder()
    .js(deno_core::include_js_files!(
      prefix "deno:ext/flash",
      "00_serve.js",
    ))
    .ops(vec![op_flash_start::decl(), op_flash_try_write::decl()])
    .state(move |op_state| {
      op_state.put(Unstable(unstable));
      Ok(())
    })
    .build()
}
