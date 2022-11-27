// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

use deno_core::include_js_files;
use deno_core::op;
use deno_core::Extension;
use rand::{rngs::StdRng, thread_rng, Rng, SeedableRng};
use std::cell::RefCell;
use std::path::PathBuf;

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_NOSYS: i32 = 52;

thread_local!(static RNG: RefCell<StdRng>  = RefCell::new(StdRng::from_entropy()));

pub fn init() -> Extension {
  Extension::builder()
    .js(include_js_files!(
      prefix "deno:ext/wasi",
      "00_wasi.js",
    ))
    .build()
}

#[op(wasm)]
fn op_args_get(
  argv_offset: i32,
  argv_buffer_offset: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op(wasm)]
fn op_args_sizes_get(
  argc_offset: i32,
  argv_buffer_size_offset: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op(wasm)]
fn op_environ_get(
  environ_offset: i32,
  environ_buffer_offset: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op(wasm)]
fn op_environ_sizes_get(
  environ_count_offset: i32,
  environ_size_offset: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op(wasm)]
fn op_clock_res_get(
  clock_id: i32,
  resolution: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op(wasm)]
fn op_clock_time_get(
  clock_id: i32,
  precision: u64,
  time: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_SUCCESS
}

#[op]
fn op_fd_advise(_fd: i32, _offset: u64, _len: u64, _advice: i32) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_allocate(_fd: i32, _offset: u64, _len: u64) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_close(_fd: i32) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_datasync(_fd: i32) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_fdstat_get(_fd: i32, _buf: i32, memory: Option<&mut [u8]>) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_fdstat_set_flags(_fd: i32, _flags: i32) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_fdstat_set_rights(
  _fd: i32,
  _fs_rights_base: u64,
  _fs_rights_inheriting: u64,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_filestat_get(_fd: i32, _buf: i32, memory: Option<&mut [u8]>) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_filestat_set_size(_fd: i32, _size: u64) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_filestat_set_times(
  _fd: i32,
  _atim: u64,
  _mtim: u64,
  _fst_flags: i32,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_pread(
  _fd: i32,
  _iovs: i32,
  _iovs_len: i32,
  _offset: u64,
  _nread: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_prestat_get(_fd: i32, _buf: i32, memory: Option<&mut [u8]>) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_prestat_dir_name(
  _fd: i32,
  _path: i32,
  _path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_pwrite(
  _fd: i32,
  _ciovs: i32,
  _ciovs_len: i32,
  _offset: u64,
  _nwritten: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_read(
  _fd: i32,
  _iovs: i32,
  _iovs_len: i32,
  _nread: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_readdir(
  _fd: i32,
  _buf: i32,
  _buf_len: i32,
  _cookie: i64,
  _bufused: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_renumber(_from: i32, _to: i32) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_seek(
  _fd: i32,
  _offset: i64,
  _whence: i32,
  _newoffset: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_fd_sync(_fd: i32) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_tell(_fd: i32, _offset: i32, memory: Option<&mut [u8]>) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_fd_write(
  _fd: i32,
  _ciovs: i32,
  _ciovs_len: i32,
  _nwritten: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_create_directory(
  _fd: i32,
  _path_ptr: i32,
  _path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_filestat_get(
  _fd: i32,
  _flags: i32,
  _path_ptr: i32,
  _path_len: i32,
  _buf: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
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
}

#[op(wasm)]
fn op_path_link(
  _old_fd: i32,
  _old_flags: i32,
  _old_path_ptr: i32,
  _old_path_len: i32,
  _new_fd: i32,
  _new_path_ptr: i32,
  _new_path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
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
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_readlink(
  _fd: i32,
  _path_ptr: i32,
  _path_len: i32,
  _buf: i32,
  _buf_len: i32,
  _bufused: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_remove_directory(
  _fd: i32,
  _path_ptr: i32,
  _path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_rename(
  _old_fd: i32,
  _old_path_ptr: i32,
  _old_path_len: i32,
  _new_fd: i32,
  _new_path_ptr: i32,
  _new_path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_symlink(
  _old_path_ptr: i32,
  _old_path_len: i32,
  _fd: i32,
  _new_path_ptr: i32,
  _new_path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op(wasm)]
fn op_path_unlink_file(
  _fd: i32,
  _path_ptr: i32,
  _path_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_poll_oneoff() -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_proc_exit(rval: i32) {
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

#[op(wasm)]
fn op_random_get(
  buffer_offset: i32,
  buffer_len: i32,
  memory: Option<&mut [u8]>,
) -> i32 {
  let mem = memory.unwrap();
  RNG.with(|rng| {
    let rng = &mut rng.borrow_mut();
    rng.fill(
      &mut mem[buffer_offset as usize..(buffer_offset + buffer_len) as usize],
    )
  });

  ERRNO_SUCCESS
}

#[op]
fn op_sock_recv(
  _fd: i32,
  _riDataOffset: i32,
  _riDataLength: i32,
  _riFlags: i32,
  _roDataLengthOffset: i32,
  _roFlagsOffset: i32,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_sock_send(
  _fd: i32,
  _siDataOffset: i32,
  _siDataLength: i32,
  _siFlags: i32,
  _soDataLengthOffset: i32,
) -> i32 {
  ERRNO_NOSYS
}

#[op]
fn op_sock_shutdown(_fd: i32, _how: i32) -> i32 {
  ERRNO_NOSYS
}

pub fn get_declaration() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib.deno_wasi.d.ts")
}
