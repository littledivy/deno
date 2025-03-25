use deno_core::error::ResourceError;
use deno_core::op2;
use deno_core::unsync::spawn;
use deno_core::v8;
use deno_core::GarbageCollected;
use deno_core::OpState;
use deno_core::ResourceId;
use tokio::task::yield_now;

pub struct HandleWrap {
  handle: ResourceId,
}

impl GarbageCollected for HandleWrap {}

impl HandleWrap {
  pub(crate) fn new(handle: ResourceId) -> Self {
    Self { handle }
  }
}

#[op2]
impl HandleWrap {
  fn close(
    &self,
    #[this] this: v8::Global<v8::Object>,
    state: &mut OpState,
    isolate_ptr: *mut v8::Isolate,
    scope: &mut v8::HandleScope,
    #[global] cb: Option<v8::Global<v8::Function>>,
  ) -> Result<(), ResourceError> {
    let resource = state.resource_table.take_any(self.handle)?;
    let context = scope.get_current_context();

    let context = v8::Global::new(scope, context);

    spawn(async move {
      // Workaround for https://github.com/denoland/deno/pull/24656
      //
      // We need to delay 'cb' at least 2 ticks to avoid "close" event happening before "error"
      // event in net.Socket.
      //
      // This is a temporary solution. We should support async close like `uv_close(handle, close_cb)`.
      yield_now().await;
      yield_now().await;

      resource.close();

      let scope = &mut v8::HandleScope::with_context(
        // SAFETY: `isolate_ptr` is a valid pointer to an `Isolate` and spawned tasks are guaranteed
        // to never outlive.
        unsafe { &mut *isolate_ptr },
        &context,
      );

      // Call _onClose() on the JS handles. Not needed for Rust handles.
      let this = v8::Local::new(scope, this);
      let on_close_str = v8::String::new(scope, "_onClose").unwrap();
      let onclose = this.get(scope, on_close_str.into());

      if let Some(onclose) = onclose {
        let fn_: v8::Local<v8::Function> = onclose.try_into().unwrap();
        fn_.call(scope, this.into(), &[]);
      }

      if let Some(cb) = cb {
        let recv = v8::undefined(scope);
        cb.open(scope).call(scope, recv.into(), &[]);
      }
    });

    Ok(())
  }

  #[fast]
  fn has_ref(&self, state: &mut OpState) -> bool {
    !state.unrefed_resources.contains(&self.handle)
  }

  #[fast]
  #[rename("r#ref")]
  fn ref_(&self, state: &mut OpState) {
    state.unrefed_resources.insert(self.handle);
  }

  #[fast]
  fn unref(&self, state: &mut OpState) {
    state.unrefed_resources.remove(&self.handle);
  }
}
