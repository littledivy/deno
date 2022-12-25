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
use serde::Deserialize;
use serde::Serialize;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::future::Future;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::rc::Rc;
use std::time::SystemTime;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{
  unbounded_channel, UnboundedReceiver, UnboundedSender,
};

mod date;
mod event;
mod request;
mod websocket;

use date::DateLoopCancelHandle;
use date::HttpDate;
use request::Request;

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

#[derive(Debug, Clone)]
pub struct Socket {
  pub inner: Rc<RefCell<tokio::net::TcpStream>>,
}

unsafe impl Send for Socket {}
unsafe impl Sync for Socket {}

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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenOpts {
  cert: Option<String>,
  key: Option<String>,
  hostname: String,
  port: u16,
  reuseport: bool,
}

fn make_addr_port_pair(hostname: &str, port: u16) -> (&str, u16) {
  // Default to localhost if given just the port. Example: ":80"
  if hostname.is_empty() {
    return ("0.0.0.0", port);
  }

  // If this looks like an ipv6 IP address. Example: "[2001:db8::1]"
  // Then we remove the brackets.
  let addr = hostname.trim_start_matches('[').trim_end_matches(']');
  (addr, port)
}

/// Resolve network address *synchronously*.
pub fn resolve_addr_sync(
  hostname: &str,
  port: u16,
) -> Result<impl Iterator<Item = SocketAddr>, AnyError> {
  let addr_port_pair = make_addr_port_pair(hostname, port);
  let result = addr_port_pair.to_socket_addrs()?;
  Ok(result)
}

#[op(v8)]
fn op_flash_start(
  scope: &mut v8::HandleScope,
  state: Rc<RefCell<OpState>>,
  js_cb: serde_v8::Value,
  opts: ListenOpts,
) -> Result<impl Future<Output = Result<(), AnyError>>, AnyError> {
  let ListenOpts {
    reuseport,
    hostname,
    port,
    ..
  } = opts;

  let addr = resolve_addr_sync(&hostname, port)?
    .next()
    .ok_or_else(|| generic_error("No resolved address found"))?;

  let domain = if addr.is_ipv4() {
    socket2::Domain::IPV4
  } else {
    socket2::Domain::IPV6
  };
  let socket = socket2::Socket::new(domain, socket2::Type::STREAM, None)?;

  #[cfg(not(windows))]
  socket.set_reuse_address(true)?;
  if reuseport {
    #[cfg(target_os = "linux")]
    socket.set_reuse_port(true)?;
  }

  let socket_addr = socket2::SockAddr::from(addr);
  socket.bind(&socket_addr)?;
  socket.listen(128)?;
  socket.set_nonblocking(true)?;

  let std_listener: std::net::TcpListener = socket.into();
  let mut listener = TcpListener::from_std(std_listener)?;

  let js_cb = event::JsCb::new(scope, js_cb);

  // SAFETY: OpState lives as long as the isolate.
  let op_state = { &state.borrow() as &OpState as *const OpState };
  let state = SharedOpState(op_state as *mut OpState);

  // This is a Send-future but won't actually every move to
  // another thread. This runs on the FuturesUnordered sub executor
  // in JsRuntime.
  //
  // We could use a LocalSet but microbenchmarks show that it is
  // slower.
  Ok(async move {
    loop {
      let (socket, _) = listener.accept().await.unwrap();
      let socket = Socket {
        inner: Rc::new(RefCell::new(socket)),
      };

      let server_socket = unsafe { &mut *socket.inner.as_ptr() };

      tokio::task::spawn(async move {
        let mut read_buf = UnsafeCell::new(vec![0u8; 1024]);
        'outer: loop {
          let mut headers = [httparse::EMPTY_HEADER; 40];
          let mut req = httparse::Request::new(&mut headers);
          let mut offset = 0;

          loop {
            let buf = unsafe { &mut read_buf.get_mut() };
            if offset >= buf.len() {
              // Grow the buffer if we need to.
              buf.resize(offset * 2, 0);
            }

            let nread = server_socket.read(&mut buf[offset..]).await;
            match nread {
              Ok(0) => break 'outer,
              Ok(n) => {
                offset += n;

                let buf = unsafe { &mut *read_buf.get() };
                match req.parse(&buf[..offset]) {
                  Ok(httparse::Status::Complete(o)) => {
                    unsafe {
                      js_cb.call(
                        state
                          .add_resource(Request::new(socket.clone(), unsafe {
                            std::mem::transmute(req)
                          })),
                      )
                    };
                    break;
                  }
                  Ok(httparse::Status::Partial) => {}
                  Err(_) => {
                    // bad request
                    break 'outer;
                  }
                };
              }
              Err(err) => {
                println!("Error {}", err);
              }
            }
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
  let request = state.resource_table.get::<Request>(rid)?;
  Ok(request.try_write(buffer)? as u32)
}

#[op]
fn op_flash_get_headers(
  state: &mut OpState,
  rid: u32,
) -> Result<Vec<(ByteString, ByteString)>, AnyError> {
  let req = state.resource_table.get::<Request>(rid)?;

  let headers = &req.request.headers;
  Ok(
    headers
      .iter()
      .map(|h| (h.name.as_bytes().into(), h.value.into()))
      .collect(),
  )
}

#[op]
fn op_flash_try_write_status_str(
  state: &mut OpState,
  rid: u32,
  status: u32,
  data: String,
) -> Result<u32, AnyError> {
  let req = state.resource_table.take::<Request>(rid)?;
  let date = state.borrow::<HttpDate>();
  let response = format!(
    "HTTP/1.1 {} OK\r\nDate: {}\r\ncontent-type: {}\r\nContent-Length: {}\r\n\r\n{}",
    status,
    date.current_date,
    "text/plain;charset=utf-8",
    data.len(),
    data
  );
  Ok(req.try_write(response.as_bytes())? as u32)
}

pub fn init<P: FlashPermissions + 'static>(unstable: bool) -> Extension {
  Extension::builder()
    .js(deno_core::include_js_files!(
      prefix "deno:ext/flash",
      "00_serve.js",
    ))
    .ops(vec![
      op_flash_start::decl(),
      op_flash_try_write_status_str::decl(),
      op_flash_try_write::decl(),
      date::op_flash_start_date_loop::decl(),
      date::op_flash_stop_date_loop::decl(),
      request::op_flash_get_method::decl(),
      request::op_flash_get_headers::decl(),
      request::op_flash_get_url::decl(),
      // websocket
      websocket::op_flash_upgrade_websocket::decl(),
    ])
    .state(move |op_state| {
      op_state.put(Unstable(unstable));
      op_state.put(HttpDate::now());
      op_state.put(DateLoopCancelHandle(CancelHandle::new_rc()));
      Ok(())
    })
    .build()
}
