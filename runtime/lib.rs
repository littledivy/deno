// Copyright 2018-2021 the Deno authors. All rights reserved. MIT license.

#![deny(warnings)]

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;

#[cfg(feature = "crypto")]
pub use deno_crypto;
#[cfg(feature = "fetch")]
pub use deno_fetch;
pub use deno_web;
#[cfg(feature = "websocket")]
pub use deno_websocket;

pub mod colors;
pub mod errors;
pub mod fs_util;
#[cfg(feature = "net")]
pub mod http_util;
#[cfg(feature = "inspector")]
pub mod inspector;
pub mod js;
pub mod metrics;
pub mod ops;
pub mod permissions;
pub mod resolve_addr;
pub mod tokio_util;
pub mod web_worker;
pub mod worker;
