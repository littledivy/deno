// Copyright 2018-2026 the Deno authors. MIT license.

use std::borrow::Cow;
use std::rc::Rc;

use deno_core::CancelFuture;
use deno_core::OpState;
use deno_core::futures::FutureExt;
use deno_core::url::Url;
use deno_fs::FileSystemRc;
use deno_fs::OpenOptions;
use deno_io::fs::FileResource;
use deno_permissions::CheckedPath;
use deno_permissions::OpenAccessKind;
use deno_permissions::PermissionsContainer;
use http_body_util::combinators::BoxBody;

use crate::CancelHandle;
use crate::CancelableResponseFuture;
use crate::FetchHandler;
use crate::ResourceToBodyAdapter;

fn content_type_for_ext(ext: &str) -> Option<&'static str> {
  Some(match ext {
    "wasm" => "application/wasm",
    "js" | "mjs" | "cjs" => "text/javascript",
    "json" => "application/json",
    "html" | "htm" => "text/html",
    "css" => "text/css",
    "txt" => "text/plain",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "wav" => "audio/wav",
    "woff2" => "font/woff2",
    _ => return None,
  })
}

/// An implementation which tries to read file URLs via `deno_fs::FileSystem`.
#[derive(Clone)]
pub struct FsFetchHandler;

impl FetchHandler for FsFetchHandler {
  fn fetch_file(
    &self,
    state: &mut OpState,
    url: &Url,
  ) -> (CancelableResponseFuture, Option<Rc<CancelHandle>>) {
    let cancel_handle = CancelHandle::new_rc();
    let path = match deno_path_util::url_to_file_path(url) {
      Ok(path) => path,
      Err(err) => {
        return (
          async move { Err(super::FetchError::UrlToFilePath(err)) }
            .or_cancel(&cancel_handle)
            .boxed_local(),
          Some(cancel_handle),
        );
      }
    };
    // Set a Content-Type by file extension so consumers that gate on it (e.g.
    // `WebAssembly.instantiateStreaming`, which requires `application/wasm`) work
    // on `file://` responses, matching browser/deno behaviour.
    let content_type = path
      .extension()
      .and_then(|e| e.to_str())
      .and_then(content_type_for_ext);
    let fs = state.borrow::<FileSystemRc>().clone();
    let path = state
      .borrow::<PermissionsContainer>()
      .check_open(Cow::Owned(path), OpenAccessKind::Read, Some("fetch()"))
      .map(CheckedPath::into_owned);
    let response_fut = async move {
      let file = fs
        .open_async(path?, OpenOptions::read())
        .await
        .map_err(|_| super::FetchError::NetworkError)?;
      let resource = Rc::new(FileResource::new(file, "".to_owned()));
      let body = BoxBody::new(ResourceToBodyAdapter::new(resource));
      let mut builder = http::Response::builder();
      if let Some(ct) = content_type {
        builder = builder.header(http::header::CONTENT_TYPE, ct);
      }
      let response = builder
        .body(body)
        .map_err(|_| super::FetchError::NetworkError)?;
      Ok(response)
    }
    .or_cancel(&cancel_handle)
    .boxed_local();

    (response_fut, Some(cancel_handle))
  }
}
