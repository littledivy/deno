mod standalone;

mod resolver;
mod args;
mod version;
mod util;
mod file_fetcher;
mod auth_tokens;
mod http_util;
mod cache;
mod errors;
mod npm;
mod node;

mod worker;

mod emit;
mod js;

use crate::args::Flags;
pub use deno_runtime::UNSTABLE_GRANULAR_FLAGS;
use deno_runtime::colors;

pub(crate) fn unstable_exit_cb(feature: &str, api_name: &str) {
  eprintln!(
    "Unstable API '{api_name}'. The `--unstable-{}` flag must be provided.",
    feature
  );
  std::process::exit(70);
}

use std::env;
use std::env::current_exe;
use deno_runtime::tokio_util::create_and_run_current_thread_with_maybe_metrics;

fn main() {
  let args: Vec<String> = env::args().collect();
  let future = async move {
    let current_exe_path = current_exe().unwrap();
    let standalone_res =
      match standalone::extract_standalone(&current_exe_path, args.clone())
        .await
      {
        Ok(Some((metadata, eszip))) => standalone::run(eszip, metadata).await,
        Ok(None) => Ok(()),
        Err(err) => Err(err),
      };
  };

  create_and_run_current_thread_with_maybe_metrics(future);
}
