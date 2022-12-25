use deno_core::serde_v8;
use deno_core::v8;

#[derive(Clone, Copy)]
pub(crate) struct JsCb {
  isolate: *mut v8::Isolate,
  js_cb: *mut v8::Function,
  context: *mut v8::Context,
}

impl JsCb {
  pub fn new(scope: &mut v8::HandleScope, cb: serde_v8::Value) -> Self {
    let current_context = scope.get_current_context();
    let context = v8::Global::new(scope, current_context).into_raw();
    let isolate: *mut v8::Isolate = &mut *scope as &mut v8::Isolate;
    Self {
      isolate,
      js_cb: v8::Global::new(scope, cb.v8_value).into_raw().as_ptr()
        as *mut v8::Function,
      context: context.as_ptr(),
    }
  }

  // SAFETY: Must be called from the same thread as the isolate.
  pub unsafe fn call(&self, rid: u32) {
    let js_cb = unsafe { &mut *self.js_cb };
    let isolate = unsafe { &mut *self.isolate };
    let context = unsafe {
      std::mem::transmute::<*mut v8::Context, v8::Local<v8::Context>>(
        self.context,
      )
    };
    let recv = v8::undefined(isolate).into();
    let scope = &mut v8::HandleScope::with_context(isolate, context);
    let args = &[v8::Integer::new(scope, rid as i32).into()];
    let _ = js_cb.call(scope, recv, args);
  }
}

// SAFETY: JsCb is Send + Sync to bypass restrictions in tokio::spawn.
unsafe impl Send for JsCb {}
unsafe impl Sync for JsCb {}
