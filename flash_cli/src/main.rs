use deno_core::JsRuntime;
use std::env;

struct Permissions;

impl deno_flash2::FlashPermissions for Permissions {
  fn check_net<T: AsRef<str>>(
    &mut self,
    _host: &(T, Option<u16>),
    _api_name: &str,
  ) -> Result<(), deno_core::error::AnyError> {
    Ok(())
  }
}

fn main() {
  // NOTE: `--help` arg will display V8 help and exit
  deno_core::v8_set_flags(env::args().collect());

  let mut js_runtime = JsRuntime::new(deno_core::RuntimeOptions {
    extensions: vec![deno_flash2::init::<Permissions>(true)],
    ..Default::default()
  });

  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();

  let future = async move {
    js_runtime
      .execute_script("flash.js", include_str!("flash.js"))
      .unwrap();
    js_runtime.run_event_loop(false).await
  };
  runtime.block_on(future).unwrap();
}
