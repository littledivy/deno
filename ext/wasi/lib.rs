// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

use deno_core::error::{type_error, AnyError};
use deno_core::include_js_files;
use deno_core::op;
use deno_core::serde_v8;
use deno_core::v8;
use deno_core::Extension;
use deno_core::OpState;
use deno_core::ResourceId;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::borrow::Cow;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::transmute;
use std::path::PathBuf;
use std::ptr::NonNull;

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_NOSYS: i32 = 52;

const FILETYPE_CHARACTER_DEVICE: i32 = 2;
const FDFLAGS_APPEND: i32 = 0x0001;

thread_local!(static RNG: RefCell<StdRng>  = RefCell::new(StdRng::from_entropy()));

struct FileDescriptor {
  rid: ResourceId,
  type_: i32,
  flags: i32,
}

struct WasiContext {
  /// An array of strings that the WebAssembly instance will see as command-line
  /// arguments.
  ///
  /// The first argument is the virtual path to the command itself.
  args: Vec<String>,
  /// An object of string keys mapped to string values that the WebAssembly module
  /// will see as its environment.
  env: HashMap<String, String>,
  /// Determines if calls to exit from within the WebAssembly module will terminate
  /// the proess or return.
  exit_on_return: bool,
  fds: Vec<FileDescriptor>,
  memory: Cell<Option<NonNull<v8::WasmMemoryObject>>>,
}

impl deno_core::Resource for WasiContext {
  fn name(&self) -> Cow<str> {
    "wasiContext".into()
  }
}

#[op]
fn op_wasi_create(
  state: &mut OpState,
  args: Vec<String>,
  env: HashMap<String, String>,
  exit_on_return: bool,
  stdin: ResourceId,
  stdout: ResourceId,
  stderr: ResourceId,
) -> ResourceId {
  let ctx = WasiContext {
    args,
    env,
    exit_on_return,
    fds: vec![
      FileDescriptor {
        rid: stdin,
        type_: FILETYPE_CHARACTER_DEVICE,
        flags: FDFLAGS_APPEND,
      },
      FileDescriptor {
        rid: stdout,
        type_: FILETYPE_CHARACTER_DEVICE,
        flags: FDFLAGS_APPEND,
      },
      FileDescriptor {
        rid: stderr,
        type_: FILETYPE_CHARACTER_DEVICE,
        flags: FDFLAGS_APPEND,
      },
    ],
    memory: Cell::new(None),
  };
  state.resource_table.add(ctx)
}

#[op(v8)]
fn op_wasi_set_memory(
  scope: &mut v8::HandleScope,
  state: &mut OpState,
  rid: ResourceId,
  memory: serde_v8::Value,
) -> Result<(), AnyError> {
  let ctx = state.resource_table.get::<WasiContext>(rid)?;

  let memory =
    v8::Local::<v8::WasmMemoryObject>::try_from(memory.v8_value).unwrap();
  let global = v8::Global::new(scope, memory).into_raw();

  ctx.memory.set(Some(global));
  Ok(())
}

fn get_memory_fallback(
  state: &mut OpState,
  rid: ResourceId,
) -> Result<&mut [u8], AnyError> {
  let ctx = state.resource_table.get::<WasiContext>(rid)?;
  let global = ctx
    .memory
    .get()
    .ok_or_else(|| type_error("Memory not set for WASI context"))?;

  // SAFETY: `v8::Local` is always non-null pointer; the `HandleScope` is
  // already on the stack, but we don't have access to it.
  let memory_object = unsafe {
    transmute::<NonNull<v8::WasmMemoryObject>, v8::Local<v8::WasmMemoryObject>>(
      global,
    )
  };

  let backing_store = memory_object.buffer().get_backing_store();
  let ptr = backing_store.data().unwrap().as_ptr() as *mut u8;
  let len = backing_store.byte_length();
  // SAFETY: `ptr` is a valid pointer to `len` bytes.
  Ok(unsafe { std::slice::from_raw_parts_mut(ptr, len) })
}

macro_rules! wasi {
  (
    $memory_arg:ident,
    $(fn $name:ident(
      $( $arg:ident : $arg_ty:ty ),*,
    ) -> $ret:ty {
      $($body:tt)*
    }),* $(,)?
  ) => {
    fn wasm_ops() -> Vec<deno_core::OpDecl> {
      vec![$($name::decl(),)*]
    }

    $(
      #[op(wasm)]
      fn $name(
        state: &mut OpState,
        rid: ResourceId,
        $($arg: $arg_ty),*,
        $memory_arg: Option<&mut [u8]>,
      ) -> $ret {
        let $memory_arg = $memory_arg.unwrap_or_else(|| get_memory_fallback(state, rid).unwrap());
        $($body)*
      }
    )*
  }
}

wasi! {
  memory,
  fn op_args_get(
    argv_offset: i32,
    argv_buffer_offset: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_args_sizes_get(
    argc_offset: i32,
    argv_buffer_size_offset: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_environ_get(
    environ_offset: i32,
    environ_buffer_offset: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_environ_sizes_get(
    environ_count_offset: i32,
    environ_size_offset: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_clock_res_get(
    clock_id: i32,
    resolution: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_clock_time_get(
    clock_id: i32,
    precision: u64,
    time: i32,
  ) -> i32 {
    ERRNO_SUCCESS
  },

  fn op_fd_fdstat_get(_fd: i32, _buf: i32, ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_pread(
    _fd: i32,
    _iovs: i32,
    _iovs_len: i32,
    _offset: u64,
    _nread: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_prestat_get(_fd: i32, _buf: i32, ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_prestat_dir_name(
    _fd: i32,
    _path: i32,
    _path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_pwrite(
    _fd: i32,
    _ciovs: i32,
    _ciovs_len: i32,
    _offset: u64,
    _nwritten: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_read(
    _fd: i32,
    _iovs: i32,
    _iovs_len: i32,
    _nread: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_readdir(
    _fd: i32,
    _buf: i32,
    _buf_len: i32,
    _cookie: i64,
    _bufused: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_seek(
    _fd: i32,
    _offset: i64,
    _whence: i32,
    _newoffset: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_fd_tell(_fd: i32, _offset: i32, ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_write(
    _fd: i32,
    _ciovs: i32,
    _ciovs_len: i32,
    _nwritten: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_path_create_directory(
    _fd: i32,
    _path_ptr: i32,
    _path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_path_filestat_get(
    _fd: i32,
    _flags: i32,
    _path_ptr: i32,
    _path_len: i32,
    _buf: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_link(
    _old_fd: i32,
    _old_flags: i32,
    _old_path_ptr: i32,
    _old_path_len: i32,
    _new_fd: i32,
    _new_path_ptr: i32,
    _new_path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_open(
    _fd: i32,
    _dirflags: i32,
    _path_ptr: i32,
    _path_len: i32,
    _oflags: i32,
    _fs_rights_base: u64,
    _fs_rights_inheriting: u64,
    _fdflags: i32,
    _opened_fd: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_readlink(
    _fd: i32,
    _path_ptr: i32,
    _path_len: i32,
    _buf: i32,
    _buf_len: i32,
    _bufused: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_path_remove_directory(
    _fd: i32,
    _path_ptr: i32,
    _path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_path_rename(
    _old_fd: i32,
    _old_path_ptr: i32,
    _old_path_len: i32,
    _new_fd: i32,
    _new_path_ptr: i32,
    _new_path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_symlink(
    _old_path_ptr: i32,
    _old_path_len: i32,
    _fd: i32,
    _new_path_ptr: i32,
    _new_path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_unlink_file(
    _fd: i32,
    _path_ptr: i32,
    _path_len: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_random_get(
    buffer_offset: i32,
    buffer_len: i32,
  ) -> i32 {
    RNG.with(|rng| {
      let rng = &mut rng.borrow_mut();
      rng.fill(
        &mut memory[buffer_offset as usize..(buffer_offset + buffer_len) as usize],
      )
    },);

    ERRNO_SUCCESS
  },

  // Non-memory ops

  fn op_fd_renumber(_from: i32, _to: i32,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_fdstat_set_flags(_fd: i32, _flags: i32,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_fdstat_set_rights(
    _fd: i32,
    _fs_rights_base: u64,
    _fs_rights_inheriting: u64,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_filestat_get(_fd: i32, _buf: i32, ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_filestat_set_size(_fd: i32, _size: u64,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_filestat_set_times(
    _fd: i32,
    _atim: u64,
    _mtim: u64,
    _fst_flags: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_advise(_fd: i32, _offset: u64, _len: u64, _advice: i32,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_allocate(_fd: i32, _offset: u64, _len: u64,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_close(_fd: i32,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_datasync(_fd: i32,) -> i32 {
    ERRNO_NOSYS
  },


  fn op_fd_sync(_fd: i32,) -> i32 {
    ERRNO_NOSYS
  },

  fn op_path_filestat_set_times(
    _fd: i32,
    _flags: i32,
    _path_ptr: i32,
    _path_len: i32,
    _atim: u64,
    _mtim: u64,
    _fst_flags: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_sock_recv(
    _fd: i32,
    _riDataOffset: i32,
    _riDataLength: i32,
    _riFlags: i32,
    _roDataLengthOffset: i32,
    _roFlagsOffset: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_sock_send(
    _fd: i32,
    _siDataOffset: i32,
    _siDataLength: i32,
    _siFlags: i32,
    _soDataLengthOffset: i32,
  ) -> i32 {
    ERRNO_NOSYS
  },

  fn op_sock_shutdown(_fd: i32, _how: i32,) -> i32 {
    ERRNO_NOSYS
  },
}

#[op]
fn op_poll_oneoff() -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_proc_exit(rval: i32) -> i32 {
  std::process::exit(0);
}

#[op]
fn op_proc_raise() -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_sched_yield() -> i32 {
  ERRNO_SUCCESS
}

pub fn init() -> Extension {
  let mut ops = wasm_ops();
  ops.extend([
    op_poll_oneoff::decl(),
    op_proc_exit::decl(),
    op_proc_raise::decl(),
    op_sched_yield::decl(),
    op_wasi_create::decl(),
    op_wasi_set_memory::decl(),
  ]);

  Extension::builder()
    .js(include_js_files!(
      prefix "deno:ext/wasi",
      "00_wasi.js",
    ))
    .ops(wasm_ops())
    .build()
}

pub fn get_declaration() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib.deno_wasi.d.ts")
}
