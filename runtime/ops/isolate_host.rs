// Copyright 2018-2026 the Deno authors. MIT license.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use deno_core::JsRuntime;
use deno_core::ModuleLoadOptions;
use deno_core::ModuleLoadReferrer;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleSpecifier;
use deno_core::OpState;
use deno_core::PollEventLoopOptions;
use deno_core::ResolutionKind;
use deno_core::error::ModuleLoaderError;
use deno_core::op2;
use deno_core::serde::Deserialize;
use deno_core::serde::Serialize;
use deno_core::v8;
use deno_permissions::ChildPermissionsArg;
use deno_permissions::PermissionsContainer;
use log::debug;

use crate::tokio_util::create_and_run_current_thread;

// ── Isolate ID ──────────────────────────────────────────────────────────

static ISOLATE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IsolateId(u32);

impl IsolateId {
  pub fn new() -> Self {
    let id = ISOLATE_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    IsolateId(id)
  }
}

impl std::fmt::Display for IsolateId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "isolate-{}", self.0)
  }
}

// ── Channel types ───────────────────────────────────────────────────────

/// Request sent from host to the isolate thread.
pub enum IsolateRequest {
  /// Evaluate a JS expression and return the JSON-serialized result.
  Eval {
    code: String,
    response_tx: tokio::sync::oneshot::Sender<Result<String, String>>,
  },
  /// Load and execute a file as the main module.
  ExecFile {
    specifier: String,
    response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
  },
  /// Resolve an npm package to its entry point and execute it.
  ExecNpm {
    package: String,
    response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
  },
  /// Shut down the isolate.
  Close,
}

/// Handle that the host thread uses to communicate with the isolate.
#[derive(Clone)]
pub struct IsolateHandle {
  request_tx: tokio::sync::mpsc::UnboundedSender<IsolateRequest>,
  isolate_handle: v8::IsolateHandle,
  terminated: Arc<AtomicBool>,
}

impl IsolateHandle {
  pub fn terminate(&self) {
    let already = self.terminated.swap(true, Ordering::SeqCst);
    if !already {
      self.isolate_handle.terminate_execution();
    }
    let _ = self.request_tx.send(IsolateRequest::Close);
  }
}

/// Sendable version created on the isolate thread, sent to the host.
pub struct SendableIsolateHandle {
  request_tx: tokio::sync::mpsc::UnboundedSender<IsolateRequest>,
  isolate_handle: v8::IsolateHandle,
  terminated: Arc<AtomicBool>,
}

impl From<SendableIsolateHandle> for IsolateHandle {
  fn from(h: SendableIsolateHandle) -> Self {
    IsolateHandle {
      request_tx: h.request_tx,
      isolate_handle: h.isolate_handle,
      terminated: h.terminated,
    }
  }
}

// ── Isolate thread bookkeeping ──────────────────────────────────────────

pub struct IsolateThread {
  handle: IsolateHandle,
}

impl Drop for IsolateThread {
  fn drop(&mut self) {
    self.handle.terminate();
  }
}

pub type IsolatesTable = HashMap<IsolateId, IsolateThread>;

// ── Resource limits ─────────────────────────────────────────────────────

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IsolateResourceLimits {
  /// Maximum memory in megabytes.
  pub memory_limit_mb: Option<usize>,
  /// CPU timeout in milliseconds.
  pub cpu_timeout_ms: Option<u64>,
}

// ── Create callback ─────────────────────────────────────────────────────

/// Arguments passed from JS to create an isolate.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIsolateArgs {
  pub permissions: Option<ChildPermissionsArg>,
  pub resources: Option<IsolateResourceLimits>,
  /// List of allowed builtin modules (e.g. ["node:fs", "node:path"]).
  /// If omitted, all builtins are available.
  pub builtins: Option<Vec<String>>,
  /// Whether eval() / new Function() are allowed. Default: false.
  #[serde(default)]
  pub eval_allowed: bool,
  /// Whether the isolate can create sub-isolates. Default: true.
  #[serde(default = "default_true")]
  pub nest: bool,
}

fn default_true() -> bool {
  true
}

// ── Callback for creating isolate runtime ──────────────────────────────

pub struct CreateIsolateRuntimeArgs {
  pub parent_permissions: PermissionsContainer,
  pub permissions: PermissionsContainer,
  pub resource_limits: Option<IsolateResourceLimits>,
  pub builtins: Option<Vec<String>>,
  pub eval_allowed: bool,
  pub nest: bool,
}

/// Callback to create an isolate's JsRuntime. This is provided by the CLI
/// crate which has access to the full module loader, npm resolver, etc.
pub type CreateIsolateRuntimeCb =
  dyn Fn(CreateIsolateRuntimeArgs) -> JsRuntime + Sync + Send;

#[derive(Clone)]
pub struct CreateIsolateRuntimeCbHolder(pub Arc<CreateIsolateRuntimeCb>);

// ── FilteredModuleLoader ───────────────────────────────────────────────

/// A module loader wrapper that gates access to `node:*` builtin modules
/// based on an allowed set. If `allowed_builtins` is `None`, all modules
/// are permitted (no filtering). If `Some`, only the listed `node:*`
/// specifiers can be resolved/loaded — others produce an error.
pub struct FilteredModuleLoader {
  inner: Rc<dyn ModuleLoader>,
  allowed_builtins: Option<HashSet<String>>,
}

impl FilteredModuleLoader {
  pub fn new(
    inner: Rc<dyn ModuleLoader>,
    builtins: Option<Vec<String>>,
  ) -> Self {
    Self {
      inner,
      allowed_builtins: builtins.map(|v| v.into_iter().collect()),
    }
  }

  fn check_builtin(&self, specifier: &str) -> Result<(), ModuleLoaderError> {
    if let Some(ref allowed) = self.allowed_builtins {
      if specifier.starts_with("node:") && !allowed.contains(specifier) {
        return Err(deno_error::JsErrorBox::new(
          "Error",
          format!(
            "Module \"{specifier}\" is not available in this isolate. \
             Allowed builtins: {:?}",
            allowed
          ),
        ));
      }
    }
    Ok(())
  }
}

impl ModuleLoader for FilteredModuleLoader {
  fn resolve(
    &self,
    specifier: &str,
    referrer: &str,
    kind: ResolutionKind,
  ) -> Result<ModuleSpecifier, ModuleLoaderError> {
    self.check_builtin(specifier)?;
    self.inner.resolve(specifier, referrer, kind)
  }

  fn load(
    &self,
    module_specifier: &ModuleSpecifier,
    maybe_referrer: Option<&ModuleLoadReferrer>,
    options: ModuleLoadOptions,
  ) -> ModuleLoadResponse {
    // Also check on load in case the specifier was resolved through
    // a different path (e.g. bare specifier mapped to node:*)
    if let Err(e) = self.check_builtin(module_specifier.as_str()) {
      return ModuleLoadResponse::Sync(Err(e));
    }
    self.inner.load(module_specifier, maybe_referrer, options)
  }

  fn prepare_load(
    &self,
    module_specifier: &ModuleSpecifier,
    maybe_referrer: Option<String>,
    maybe_content: Option<String>,
    options: ModuleLoadOptions,
  ) -> Pin<Box<dyn Future<Output = Result<(), ModuleLoaderError>>>> {
    self.inner.prepare_load(
      module_specifier,
      maybe_referrer,
      maybe_content,
      options,
    )
  }

  fn finish_load(&self) {
    self.inner.finish_load()
  }

  fn code_cache_ready(
    &self,
    specifier: ModuleSpecifier,
    hash: u64,
    code_cache: &[u8],
  ) -> Pin<Box<dyn Future<Output = ()>>> {
    self.inner.code_cache_ready(specifier, hash, code_cache)
  }

  fn purge_and_prevent_code_cache(&self, specifier: &str) {
    self.inner.purge_and_prevent_code_cache(specifier)
  }

  fn get_source_map(&self, file_name: &str) -> Option<Cow<'_, [u8]>> {
    self.inner.get_source_map(file_name)
  }

  fn get_source_mapped_source_line(
    &self,
    file_name: &str,
    line_number: usize,
  ) -> Option<String> {
    self
      .inner
      .get_source_mapped_source_line(file_name, line_number)
  }

  fn get_host_defined_options<'s, 'i>(
    &self,
    scope: &mut v8::PinScope<'s, 'i>,
    name: &str,
  ) -> Option<v8::Local<'s, v8::Data>> {
    self.inner.get_host_defined_options(scope, name)
  }
}

// ── Extension ──────────────────────────────────────────────────────────

deno_core::extension!(
  deno_isolate_host,
  ops = [
    op_create_isolate,
    op_isolate_eval,
    op_isolate_exec_file,
    op_isolate_exec_npm,
    op_isolate_close,
  ],
  options = {
    create_isolate_runtime_cb: Arc<CreateIsolateRuntimeCb>,
  },
  state = |state, options| {
    state.put::<IsolatesTable>(IsolatesTable::default());
    state.put::<CreateIsolateRuntimeCbHolder>(
      CreateIsolateRuntimeCbHolder(options.create_isolate_runtime_cb),
    );
  },
);

// ── Errors ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum IsolateError {
  #[class(inherit)]
  #[error(transparent)]
  Permission(deno_permissions::ChildPermissionError),
  #[class(inherit)]
  #[error("{0}")]
  Io(#[from] std::io::Error),
  #[class(generic)]
  #[error("Isolate not found: {0}")]
  NotFound(IsolateId),
  #[class(generic)]
  #[error("Isolate eval failed: {0}")]
  EvalFailed(String),
  #[class(generic)]
  #[error("Isolate exec failed: {0}")]
  ExecFailed(String),
  #[class(generic)]
  #[error("Isolate channel closed")]
  ChannelClosed,
}

// ── Ops ────────────────────────────────────────────────────────────────

#[op2]
#[serde]
fn op_create_isolate(
  state: &mut OpState,
  #[serde] args: CreateIsolateArgs,
) -> Result<IsolateId, IsolateError> {
  let parent_permissions = state.borrow_mut::<PermissionsContainer>();
  let child_permissions = if let Some(child_permissions_arg) = args.permissions
  {
    parent_permissions
      .create_child_permissions(child_permissions_arg)
      .map_err(IsolateError::Permission)?
  } else {
    parent_permissions.clone()
  };
  let parent_permissions = parent_permissions.clone();

  let create_cb = state.borrow::<CreateIsolateRuntimeCbHolder>().clone();
  let isolate_id = IsolateId::new();

  let resource_limits = args.resources.clone();
  let builtins = args.builtins.clone();
  let eval_allowed = args.eval_allowed;
  let nest = args.nest;
  let cpu_timeout_ms = args.resources.as_ref().and_then(|r| r.cpu_timeout_ms);

  let (handle_tx, handle_rx) =
    std::sync::mpsc::sync_channel::<SendableIsolateHandle>(1);

  std::thread::Builder::new()
    .name(format!("{isolate_id}"))
    .spawn(move || {
      let fut = async move {
        let mut js_runtime = (create_cb.0)(CreateIsolateRuntimeArgs {
          parent_permissions,
          permissions: child_permissions,
          resource_limits,
          builtins,
          eval_allowed,
          nest,
        });

        let isolate_handle: v8::IsolateHandle =
          js_runtime.v8_isolate().thread_safe_handle();
        let terminated = Arc::new(AtomicBool::new(false));

        let (request_tx, mut request_rx) =
          tokio::sync::mpsc::unbounded_channel::<IsolateRequest>();

        // Send the handle back to the host
        handle_tx
          .send(SendableIsolateHandle {
            request_tx,
            isolate_handle: isolate_handle.clone(),
            terminated: terminated.clone(),
          })
          .unwrap();
        drop(handle_tx);

        // Event loop: process incoming requests
        loop {
          tokio::select! {
            biased;
            req = request_rx.recv() => {
              match req {
                Some(IsolateRequest::Eval { code, response_tx }) => {
                  let result = eval_in_runtime(&mut js_runtime, &code, cpu_timeout_ms);
                  let _ = response_tx.send(result);
                }
                Some(IsolateRequest::ExecFile { specifier, response_tx }) => {
                  let result = exec_file_in_runtime(&mut js_runtime, &specifier).await;
                  let _ = response_tx.send(result);
                }
                Some(IsolateRequest::ExecNpm { package, response_tx }) => {
                  let result = exec_npm_in_runtime(&mut js_runtime, &package).await;
                  let _ = response_tx.send(result);
                }
                Some(IsolateRequest::Close) | None => {
                  break;
                }
              }
            }
          }
        }
      };

      let _ = create_and_run_current_thread(fut);

      // Trim memory on Linux after isolate teardown
      #[cfg(target_os = "linux")]
      {
        // SAFETY: calling libc function with no preconditions.
        unsafe {
          libc::malloc_trim(0);
        }
      }
    })?;

  let sendable_handle = handle_rx.recv().map_err(|_| {
    IsolateError::Io(std::io::Error::new(
      std::io::ErrorKind::Other,
      "Failed to receive isolate handle",
    ))
  })?;

  let isolate_thread = IsolateThread {
    handle: sendable_handle.into(),
  };

  state
    .borrow_mut::<IsolatesTable>()
    .insert(isolate_id, isolate_thread);

  Ok(isolate_id)
}

#[op2]
#[string]
async fn op_isolate_eval(
  state: Rc<RefCell<OpState>>,
  #[serde] id: IsolateId,
  #[string] code: String,
) -> Result<String, IsolateError> {
  let handle = get_isolate_handle(&state, id)?;

  let (tx, rx) = tokio::sync::oneshot::channel();
  handle
    .request_tx
    .send(IsolateRequest::Eval {
      code,
      response_tx: tx,
    })
    .map_err(|_| IsolateError::ChannelClosed)?;

  rx.await
    .map_err(|_| IsolateError::ChannelClosed)?
    .map_err(IsolateError::EvalFailed)
}

#[op2]
async fn op_isolate_exec_file(
  state: Rc<RefCell<OpState>>,
  #[serde] id: IsolateId,
  #[string] specifier: String,
) -> Result<(), IsolateError> {
  let handle = get_isolate_handle(&state, id)?;

  let (tx, rx) = tokio::sync::oneshot::channel();
  handle
    .request_tx
    .send(IsolateRequest::ExecFile {
      specifier,
      response_tx: tx,
    })
    .map_err(|_| IsolateError::ChannelClosed)?;

  rx.await
    .map_err(|_| IsolateError::ChannelClosed)?
    .map_err(IsolateError::ExecFailed)
}

#[op2]
async fn op_isolate_exec_npm(
  state: Rc<RefCell<OpState>>,
  #[serde] id: IsolateId,
  #[string] package: String,
) -> Result<(), IsolateError> {
  let handle = get_isolate_handle(&state, id)?;

  let (tx, rx) = tokio::sync::oneshot::channel();
  handle
    .request_tx
    .send(IsolateRequest::ExecNpm {
      package,
      response_tx: tx,
    })
    .map_err(|_| IsolateError::ChannelClosed)?;

  rx.await
    .map_err(|_| IsolateError::ChannelClosed)?
    .map_err(IsolateError::ExecFailed)
}

#[op2]
fn op_isolate_close(state: &mut OpState, #[serde] id: IsolateId) {
  if let Some(thread) = state.borrow_mut::<IsolatesTable>().remove(&id) {
    thread.handle.terminate();
    debug!("isolate {} closed", id);
  } else {
    debug!("tried to close non-existent isolate {}", id);
  }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn get_isolate_handle(
  state: &Rc<RefCell<OpState>>,
  id: IsolateId,
) -> Result<IsolateHandle, IsolateError> {
  let s = state.borrow();
  let table = s.borrow::<IsolatesTable>();
  table
    .get(&id)
    .map(|t| t.handle.clone())
    .ok_or(IsolateError::NotFound(id))
}

fn eval_in_runtime(
  js_runtime: &mut JsRuntime,
  code: &str,
  cpu_timeout_ms: Option<u64>,
) -> Result<String, String> {
  // Optionally set a CPU timeout
  let _timeout_guard = cpu_timeout_ms.map(|ms| {
    let handle: v8::IsolateHandle =
      js_runtime.v8_isolate().thread_safe_handle();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
      if rx.recv_timeout(Duration::from_millis(ms)).is_err() {
        handle.terminate_execution();
      }
    });
    tx
  });

  let result = js_runtime.execute_script(
    "<isolate_eval>",
    deno_core::FastString::from(code.to_string()),
  );

  match result {
    Ok(global) => {
      deno_core::scope!(scope, js_runtime);
      let local = v8::Local::new(scope, &global);
      match v8::json::stringify(scope, local) {
        Some(s) => {
          let rust_str: String = s.to_rust_string_lossy(scope);
          Ok(rust_str)
        }
        None => Ok("undefined".to_string()),
      }
    }
    Err(err) => Err(err.to_string()),
  }
}

/// Load and execute a file as the main ES module inside the isolate.
async fn exec_file_in_runtime(
  js_runtime: &mut JsRuntime,
  specifier: &str,
) -> Result<(), String> {
  let module_specifier = deno_core::resolve_url_or_path(
    specifier,
    &std::env::current_dir().unwrap(),
  )
  .map_err(|e| e.to_string())?;

  let mod_id = js_runtime
    .load_main_es_module(&module_specifier)
    .await
    .map_err(|e| e.to_string())?;

  let mut receiver = js_runtime.mod_evaluate(mod_id);

  tokio::select! {
    biased;
    maybe_result = &mut receiver => {
      maybe_result.map_err(|e| e.to_string())
    }
    event_loop_result = js_runtime.run_event_loop(PollEventLoopOptions::default()) => {
      event_loop_result.map_err(|e| e.to_string())?;
      receiver.await.map_err(|e| e.to_string())
    }
  }
}

/// Resolve an npm package specifier and execute its entry point.
/// The package string can be:
///   - "package-name" → resolved as "npm:package-name"
///   - "npm:package-name" → used as-is
///   - "@scope/package" → resolved as "npm:@scope/package"
async fn exec_npm_in_runtime(
  js_runtime: &mut JsRuntime,
  package: &str,
) -> Result<(), String> {
  // Normalize to an npm: specifier
  let npm_specifier = if package.starts_with("npm:") {
    package.to_string()
  } else {
    format!("npm:{package}")
  };

  let module_specifier = ModuleSpecifier::parse(&npm_specifier)
    .map_err(|e| format!("Invalid npm specifier \"{npm_specifier}\": {e}"))?;

  let mod_id = js_runtime
    .load_main_es_module(&module_specifier)
    .await
    .map_err(|e| format!("Failed to load npm package \"{package}\": {e}"))?;

  let mut receiver = js_runtime.mod_evaluate(mod_id);

  tokio::select! {
    biased;
    maybe_result = &mut receiver => {
      maybe_result.map_err(|e| e.to_string())
    }
    event_loop_result = js_runtime.run_event_loop(PollEventLoopOptions::default()) => {
      event_loop_result.map_err(|e| e.to_string())?;
      receiver.await.map_err(|e| e.to_string())
    }
  }
}
