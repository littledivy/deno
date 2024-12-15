// Copyright 2018-2024 the Deno authors. All rights reserved. MIT license.

use deno_core::op2;
use deno_core::GarbageCollected;
use serde::Deserialize;

pub struct Context {}

impl GarbageCollected for Context {}

#[op2]
impl Context {
  #[constructor]
  #[cppgc]
  fn new(_: bool) -> Context {
    Context {}
  }

  #[fast]
  fn proc_exit(&self, #[smi] code: i32) {
    std::process::exit(code);
  }
}

pub struct WASI {}

impl GarbageCollected for WASI {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Version {
  Unstable,
  Preview1,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
  args: Vec<String>,
  return_on_exit: bool,
  stdin: i32,
  stdout: i32,
  stderr: i32,
  version: Version
}

#[op2]
impl WASI {
  #[constructor]
  #[cppgc]
  fn new(#[serde] options: Options) -> WASI {
    WASI {}
  }

  #[fast]
  fn get_import_object(&self) {
  }

  #[fast]
  fn start(&self) {}

  #[fast]
  fn initialize(&self) {}

  #[getter]
  fn wasi_imports(&self) {}
}
