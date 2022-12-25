use crate::Socket;
use deno_core::error::type_error;
use deno_core::error::AnyError;
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
