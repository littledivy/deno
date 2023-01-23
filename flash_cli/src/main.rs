use deno_core::error::AnyError;
use deno_core::url;
use deno_core::JsRuntime;
use deno_core::OpState;
use deno_core::Snapshot;
use std::env;
use std::path::Path;

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

impl deno_fetch::FetchPermissions for Permissions {
  fn check_net_url(
    &mut self,
    _url: &url::Url,
    _api_name: &str,
  ) -> Result<(), AnyError> {
    Ok(())
  }

  fn check_read(
    &mut self,
    _path: &Path,
    _api_name: &str,
  ) -> Result<(), AnyError> {
    Ok(())
  }
}

impl deno_web::TimersPermission for Permissions {
  fn allow_hrtime(&mut self) -> bool {
    true
  }

  fn check_unstable(&self, _state: &OpState, _api_name: &'static str) {
    // ...
  }
}

static FLASH_SNAPSHOT: &[u8] =
  include_bytes!(concat!(env!("OUT_DIR"), "/FLASH_SNAPSHOT.bin"));

fn main() {
  // NOTE: `--help` arg will display V8 help and exit
  deno_core::v8_set_flags(env::args().collect());

  let mut js_runtime = JsRuntime::new(deno_core::RuntimeOptions {
    extensions_with_js: vec![
      deno_webidl::init(),
      deno_url::init(),
      deno_console::init(),
      deno_web::init::<Permissions>(Default::default(), None),
      deno_fetch::init::<Permissions>(Default::default()),
      deno_flash2::init::<Permissions>(true),
    ],
    startup_snapshot: Some(Snapshot::Static(&*FLASH_SNAPSHOT)),
    ..Default::default()
  });

  {
    let state_rc = js_runtime.op_state();
    let mut state = state_rc.borrow_mut();
    state.put(Permissions);
  }
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();

  let future = async move {
    js_runtime
      .execute_script("main.js", include_str!("../main.js"))
      .unwrap();
    js_runtime.run_event_loop(false).await
  };
  runtime.block_on(future).unwrap();
}
