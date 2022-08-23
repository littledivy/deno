use deno_core::op;
use deno_core::Extension;
use deno_core::OpState;
use deno_core::v8::fast_api::FastApiCallbackOptions;

#[op(fast)]
fn op_proc_exit(code: i32) {
  std::process::exit(code);
}

#[op(fast)]
fn op_clock_res_get(options: Option<&FastApiCallbackOptions>) {}

pub fn init() -> Extension {
  Extension::builder()
    .js(deno_core::include_js_files!(
      prefix "deno:ext/wasi",
      "01_wasi.js",
    ))
    .ops(vec![op_proc_exit::decl()])
    .build()
}
