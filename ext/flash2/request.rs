use crate::Socket;
use async_http_codec::BodyDecode;
use deno_core::error::type_error;
use deno_core::error::AnyError;
use deno_core::op;
use deno_core::ByteString;
use deno_core::OpState;
use std::borrow::Cow;
use std::rc::Rc;
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct Request {
  inner: Socket,

  pub request: httparse::Request<'static, 'static>,
}

impl deno_core::Resource for Request {
  fn name(&self) -> Cow<str> {
    "httpRequest".into()
  }
}

impl Request {
  pub fn new(
    inner: Socket,
    request: httparse::Request<'static, 'static>,
  ) -> Self {
    Self { inner, request }
  }

  pub fn try_inner(self: Rc<Self>) -> Result<TcpStream, AnyError> {
    Rc::try_unwrap(self.inner.inner.clone())
      .map(|inner| inner.into_inner())
      .map_err(|_| type_error("Request has already been used".to_string()))
  }

  pub fn try_write(self: Rc<Self>, buf: &[u8]) -> Result<usize, AnyError> {
    let mut inner = self.inner.inner.borrow_mut();
    inner.try_write(buf).map_err(|err| err.into())
  }
}

macro_rules! mk_getter_op {
  ($self: ident, fn $name:ident () -> $ty:ty {
    $($body:tt)*
  }) => {
    paste::item! {
     #[op]
      pub fn [<op_flash_get_ $name>] (
        state: &mut OpState,
        rid: u32,
      ) -> Result<$ty, AnyError> {
        let $self = state.resource_table.get::<Request>(rid)?;
        Ok({
          $($body)*
        })
      }
    }
  };
}

mk_getter_op! {
  this,
  fn method() -> String {
    this.request.method.unwrap_or("").to_string()
  }
}

mk_getter_op! {
  this,
  fn url() -> String {
    this.request.path.unwrap_or("").to_string()
  }
}

mk_getter_op! {
  this,
  fn headers() -> Vec<(ByteString, ByteString)> {
    let headers = &this.request.headers;
    headers
      .iter()
      .map(|h| (h.name.as_bytes().into(), h.value.into()))
      .collect()
  }
}

mk_getter_op! {
  this,
  fn has_body() -> bool {
    this.request.method.map(|m| m != "GET" && m != "HEAD").unwrap_or(false)
  }
}
