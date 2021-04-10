// Copyright 2018-2021 the Deno authors. All rights reserved. MIT license.

use deno_core::JsRuntime;
use deno_core::RuntimeOptions;
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
// TODO(bartlomieju): this module contains a lot of duplicated
// logic with `cli/build.rs`, factor out to `deno_core`.
fn create_snapshot(
  mut js_runtime: JsRuntime,
  snapshot_path: &Path,
  files: Vec<PathBuf>,
) {
  // TODO(nayeemrmn): https://github.com/rust-lang/cargo/issues/3946 to get the
  // workspace root.
  let display_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
  for file in files {
    println!("cargo:rerun-if-changed={}", file.display());
    let display_path = file.strip_prefix(display_root).unwrap();
    let display_path_str = display_path.display().to_string();
    js_runtime
      .execute(
        &("deno:".to_string() + &display_path_str.replace('\\', "/")),
        &std::fs::read_to_string(&file).unwrap(),
      )
      .unwrap();
  }

  let snapshot = js_runtime.snapshot();
  let snapshot_slice: &[u8] = &*snapshot;
  println!("Snapshot size: {}", snapshot_slice.len());
  std::fs::write(&snapshot_path, snapshot_slice).unwrap();
  println!("Snapshot written to: {} ", snapshot_path.display());
}

fn save_private_snapshot(mut js_runtime: &JsRuntime, path: &Path) {

}

fn create_runtime_snapshot(snapshot_path: &Path, files: Vec<PathBuf>) {
  let js_runtime = JsRuntime::new(RuntimeOptions {
    will_snapshot: true,
    ..Default::default()
  });

  deno_webidl::init(&mut js_runtime);
  deno_console::init(&mut js_runtime);
  deno_url::init(&mut js_runtime);
  deno_web::init(&mut js_runtime);
  deno_file::init(&mut js_runtime);
  deno_fetch::init(&mut js_runtime);
  deno_websocket::init(&mut js_runtime);
  deno_crypto::init(&mut js_runtime);

  let changes = get_changes();
  if changes.iter().find(|c| c.starts_with("op_crates/webgpu")).is_some() {
    deno_webgpu::init(&mut js_runtime);
  }
  create_snapshot(js_runtime, snapshot_path, files);
}

fn main() {
  // Skip building from docs.rs.
  if env::var_os("DOCS_RS").is_some() {
    return;
  }

  // To debug snapshot issues uncomment:
  // op_fetch_asset::trace_serializer();

  println!("cargo:rustc-env=TARGET={}", env::var("TARGET").unwrap());
  println!("cargo:rustc-env=PROFILE={}", env::var("PROFILE").unwrap());
  let o = PathBuf::from(env::var_os("OUT_DIR").unwrap());

  // Main snapshot
  let runtime_snapshot_path = o.join("CLI_SNAPSHOT.bin");

  // WebGPU snapshot
  let wgpu_snapshot = o.join("WGPU_SNAPSHOT.bin");
  
  let js_files = get_js_files("js");
  create_runtime_snapshot(&runtime_snapshot_path, js_files);
}

fn get_js_files(d: &str) -> Vec<PathBuf> {
  let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
  let mut js_files = std::fs::read_dir(d)
    .unwrap()
    .map(|dir_entry| {
      let file = dir_entry.unwrap();
      manifest_dir.join(file.path())
    })
    .filter(|path| path.extension().unwrap_or_default() == "js")
    .collect::<Vec<PathBuf>>();
  js_files.sort();
  js_files
}

fn get_changes() -> Vec<String> {
  let output = Command::new("git")
    .args(&[
      "ls-files",
      ".",
      "-d",
      "-m",
      "-o",
      "--exclude-standard",
      "--full-name",
      "-v",
    ])
    .output()
    .unwrap();

  let changes = String::from_utf8(output.stdout);
  changes
    .split('\n')
    .map(|c| c.split_whitespace().get(1).unwrap())
}
