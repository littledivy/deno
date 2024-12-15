// Copyright 2018-2024 the Deno authors. All rights reserved. MIT license.

use deno_core::op2;
use deno_core::v8;
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
  #[rename("proc_exit")]
  fn proc_exit(&self, #[smi] code: i32) {
    println!("proc_exit code={}", code);
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
  version: Version,
}

#[op2]
impl WASI {
  #[constructor]
  #[cppgc]
  fn new(#[serde] options: Options) -> WASI {
    WASI {}
  }

  #[fast]
  fn get_import_object(&self) {}

  #[nofast]
  #[reentrant]
  fn start(
    &self,
    scope: &mut v8::HandleScope,
    instance: v8::Local<v8::Object>,
  ) {
    let exports_key = v8::String::new(scope, "exports").unwrap();
    let exports = instance.get(scope, exports_key.into()).unwrap();
    let exports_obj = v8::Local::<v8::Object>::try_from(exports).unwrap();

    let start_key = v8::String::new(scope, "_start").unwrap();

    let start = exports_obj.get(scope, start_key.into()).unwrap();

    let start_fn = v8::Local::<v8::Function>::try_from(start).unwrap();
    let null = v8::null(scope);

    let mut tc = v8::TryCatch::new(scope);

    match start_fn.call(&mut tc, null.into(), &[]) {
      None => {
        tc.rethrow();
      }
      Some(_) => {}
    }
  }

  #[fast]
  fn initialize(&self) {}

  #[cppgc]
  fn wasi_imports(&self) -> Context {
    Context {}
  }
}
