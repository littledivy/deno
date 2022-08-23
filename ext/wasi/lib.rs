use deno_core::op;
use deno_core::Extension;
use deno_core::OpState;
use deno_core::v8::fast_api::FastApiCallbackOptions;

#[inline(always)]
fn get_memory_checked<'a>(state: &mut OpState, options: Option<&FastApiCallbackOptions>) -> &'a mut [u8] {
  if let Some(options) = options {
    let memory = unsafe { &*options.wasm_memory };
    if let Some(aligned) = memory.get_storage_if_aligned() {
      return aligned;
    }
  }

  &mut [] as _ // TODO
}

#[op(fast)]
fn op_proc_exit(code: i32) {
  std::process::exit(code);
}

#[op(fast)]
fn op_clock_res_get(state: &mut OpState, options: Option<&FastApiCallbackOptions>) {

}

pub fn init() -> Extension {
  Extension::builder()
    .js(deno_core::include_js_files!(
      prefix "deno:ext/wasi",
      "01_wasi.js",
    ))
    .ops(vec![op_proc_exit::decl(), op_clock_res_get::decl()])
    .build()
}
