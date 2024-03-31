use deno_core::op2;
use deno_core::v8;
use deno_core::cppgc::make_cppgc_object;

struct Context {}

impl Context {
  pub fn new() -> Self {
    Self {}
  }

  #[op2(method, fast)]
  fn proc_exit(&self, #[smi] code: i32) {
    std::process::exit(code);
  }
}

#[op2]
pub fn op_wasi_context_new<'a>(
  ctx: &OpCtx,
  scope: &'a mut v8::HandleScope,
) -> v8::Local<'a, v8::Object> {
  let context = Context::new();
  let obj = make_cppgc_object(scope, context);
  Context::register_proc_exit(scope, ctx, obj);
  obj
}


// System sandbox primitives
mod ssp {

}

