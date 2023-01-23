use deno_core::error::AnyError;
use deno_core::include_js_files;
use deno_core::url;
use deno_core::Extension;
use deno_core::JsRuntime;
use deno_core::OpState;
use deno_core::RuntimeOptions;
use std::env;
use std::path::Path;
use std::path::PathBuf;

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

fn main() {
  let o = PathBuf::from(env::var_os("OUT_DIR").unwrap());
  let snapshot_path = o.join("FLASH_SNAPSHOT.bin");
  let mut js_runtime = JsRuntime::new(RuntimeOptions {
    will_snapshot: true,
    extensions: vec![
      deno_webidl::init(),
      deno_url::init(),
      deno_console::init(),
      deno_web::init::<Permissions>(Default::default(), None),
      deno_fetch::init::<Permissions>(Default::default()),
      deno_flash2::init::<Permissions>(true),
    ],
    ..Default::default()
  });
  let snapshot = js_runtime.snapshot();
  let snapshot_slice: &[u8] = &*snapshot;
  std::fs::write(&snapshot_path, snapshot_slice).unwrap();
}
