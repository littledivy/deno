// Copyright 2018-2026 the Deno authors. MIT license.

// Drop-in replacement for libuv integrated with deno_core's event loop.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_uint;
use std::ffi::c_void;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::time::Instant;

#[cfg(unix)]
use libc::AF_INET;
#[cfg(unix)]
use libc::AF_INET6;
#[cfg(unix)]
use libc::sockaddr_in;
#[cfg(unix)]
use libc::sockaddr_in6;
#[cfg(unix)]
type sa_family_t = libc::sa_family_t;
#[cfg(windows)]
use win_sock::AF_INET;
#[cfg(windows)]
use win_sock::AF_INET6;
#[cfg(windows)]
use win_sock::sockaddr_in;
#[cfg(windows)]
use win_sock::sockaddr_in6;
#[cfg(windows)]
type sa_family_t = win_sock::sa_family_t;

// libc doesn't export socket structs on Windows.
#[cfg(windows)]
mod win_sock {
  #[repr(C)]
  pub struct in_addr {
    pub s_addr: u32,
  }
  #[repr(C)]
  pub struct sockaddr_in {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: in_addr,
    pub sin_zero: [u8; 8],
  }
  #[repr(C)]
  pub struct in6_addr {
    pub s6_addr: [u8; 16],
  }
  #[repr(C)]
  pub struct sockaddr_in6 {
    pub sin6_family: u16,
    pub sin6_port: u16,
    pub sin6_flowinfo: u32,
    pub sin6_addr: in6_addr,
    pub sin6_scope_id: u32,
  }
  pub const AF_INET: i32 = 2;
  pub const AF_INET6: i32 = 23;
  pub type sa_family_t = u16;
  pub const SD_SEND: i32 = 1;
  unsafe extern "system" {
    pub fn shutdown(socket: usize, how: i32) -> i32;
  }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum uv_handle_type {
  UV_UNKNOWN_HANDLE = 0,
  UV_TIMER = 1,
  UV_IDLE = 2,
  UV_PREPARE = 3,
  UV_CHECK = 4,
  UV_TCP = 12,
  UV_TTY = 15,
  UV_FILE = 17,
  UV_NAMED_PIPE = 7,
}

const UV_HANDLE_ACTIVE: u32 = 1 << 0;
const UV_HANDLE_REF: u32 = 1 << 1;
const UV_HANDLE_CLOSING: u32 = 1 << 2;
const UV_HANDLE_BLOCKING_WRITES: u32 = 1 << 3;
const UV_HANDLE_TTY_READABLE: u32 = 1 << 4;

pub const UV_TTY_MODE_NORMAL: c_int = 0;
pub const UV_TTY_MODE_RAW: c_int = 1;
pub const UV_TTY_MODE_IO: c_int = 2;

// libuv-compatible error codes (negative errno values on unix,
// which vary depending on platform, fixed values on windows).
macro_rules! uv_errno {
  ($name:ident, $unix:expr, $win:expr) => {
    #[cfg(unix)]
    pub const $name: i32 = -($unix);
    #[cfg(windows)]
    pub const $name: i32 = $win;
  };
}

uv_errno!(UV_EAGAIN, libc::EAGAIN, -4088);
uv_errno!(UV_EBADF, libc::EBADF, -4083);
uv_errno!(UV_EADDRINUSE, libc::EADDRINUSE, -4091);
uv_errno!(UV_ECONNREFUSED, libc::ECONNREFUSED, -4078);
uv_errno!(UV_EINVAL, libc::EINVAL, -4071);
uv_errno!(UV_ENOTCONN, libc::ENOTCONN, -4053);
uv_errno!(UV_ECANCELED, libc::ECANCELED, -4081);
uv_errno!(UV_EPIPE, libc::EPIPE, -4047);
pub const UV_EOF: i32 = -4095;
uv_errno!(UV_ENOTSUP, libc::ENOTSUP, -4049);
uv_errno!(UV_EIO, libc::EIO, -4070);

/// Global state for `uv_tty_reset_mode` (async-signal-safe).
#[cfg(unix)]
static TTY_RESET_LOCK: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
static mut TTY_RESET_FD: c_int = -1;
#[cfg(unix)]
static mut TTY_RESET_TERMIOS: std::mem::MaybeUninit<libc::termios> =
  std::mem::MaybeUninit::uninit();

#[repr(C)]
pub struct uv_loop_t {
  internal: *mut c_void,
  pub data: *mut c_void,
  stop_flag: Cell<bool>,
}

#[repr(C)]
pub struct uv_handle_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
}

#[repr(C)]
pub struct uv_timer_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  internal_id: u64,
  internal_deadline: u64,
  cb: Option<unsafe extern "C" fn(*mut uv_timer_t)>,
  timeout: u64,
  repeat: u64,
}

#[repr(C)]
pub struct uv_idle_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  cb: Option<unsafe extern "C" fn(*mut uv_idle_t)>,
}

#[repr(C)]
pub struct uv_prepare_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  cb: Option<unsafe extern "C" fn(*mut uv_prepare_t)>,
}

#[repr(C)]
pub struct uv_check_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  cb: Option<unsafe extern "C" fn(*mut uv_check_t)>,
}

#[repr(C)]
pub struct uv_stream_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
}

#[repr(C)]
pub struct uv_tcp_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  #[cfg(unix)]
  internal_fd: Option<std::os::unix::io::RawFd>,
  #[cfg(windows)]
  internal_fd: Option<std::os::windows::io::RawSocket>,
  internal_bind_addr: Option<SocketAddr>,
  internal_stream: Option<tokio::net::TcpStream>,
  internal_listener: Option<tokio::net::TcpListener>,
  internal_listener_addr: Option<SocketAddr>,
  internal_nodelay: bool,
  internal_alloc_cb: Option<uv_alloc_cb>,
  internal_read_cb: Option<uv_read_cb>,
  internal_reading: bool,
  internal_connect: Option<ConnectPending>,
  internal_write_queue: VecDeque<WritePending>,
  internal_connection_cb: Option<uv_connection_cb>,
  internal_backlog: VecDeque<tokio::net::TcpStream>,
}

#[repr(C)]
pub struct uv_tty_t {
  pub r#type: uv_handle_type,
  pub loop_: *mut uv_loop_t,
  pub data: *mut c_void,
  pub flags: u32,
  internal_mode: c_int,
  #[cfg(unix)]
  internal_fd: std::os::unix::io::RawFd,
  #[cfg(unix)]
  internal_orig_termios: Option<libc::termios>,
  #[cfg(unix)]
  internal_async_fd:
    Option<tokio::io::unix::AsyncFd<std::os::unix::io::OwnedFd>>,
  #[cfg(windows)]
  internal_handle: *mut c_void,
  internal_alloc_cb: Option<uv_alloc_cb>,
  internal_read_cb: Option<uv_read_cb>,
  internal_reading: bool,
  internal_write_queue: VecDeque<WritePending>,
}

/// In-flight TCP connect operation.
///
/// # Safety
/// `req` is a raw pointer to a caller-owned `uv_connect_t`. The caller must
/// ensure it remains valid until the connect callback fires (at which point
/// `ConnectPending` is consumed). This struct is `!Send` -- it lives on the
/// event loop thread alongside `UvLoopInner`.
struct ConnectPending {
  future: Pin<Box<dyn Future<Output = std::io::Result<tokio::net::TcpStream>>>>,
  req: *mut uv_connect_t,
  cb: Option<uv_connect_cb>,
}

/// Queued write operation waiting for the socket to become writable.
///
/// # Safety
/// `req` is a raw pointer to a caller-owned `uv_write_t`. The caller must
/// ensure it remains valid until the write callback fires (at which point
/// `WritePending` is consumed). This struct is `!Send`.
struct WritePending {
  req: *mut uv_write_t,
  data: Vec<u8>,
  offset: usize,
  cb: Option<uv_write_cb>,
}

#[repr(C)]
pub struct uv_write_t {
  pub r#type: i32, // UV_REQ_TYPE fields
  pub data: *mut c_void,
  pub handle: *mut uv_stream_t,
}

#[repr(C)]
pub struct uv_connect_t {
  pub r#type: i32,
  pub data: *mut c_void,
  pub handle: *mut uv_stream_t,
}

#[repr(C)]
pub struct uv_shutdown_t {
  pub r#type: i32,
  pub data: *mut c_void,
  pub handle: *mut uv_stream_t,
}

/// I/O buffer descriptor matching libuv's `uv_buf_t`.
///
/// Field order is `{base, len}` which matches the macOS/Windows layout.
/// On Linux, real libuv uses `{len, base}` (matching `struct iovec`).
/// This is fine as long as the struct is only constructed/consumed in Rust;
/// if it ever needs to cross an FFI boundary to real C code on Linux,
/// the field order must be made platform-conditional.
#[repr(C)]
pub struct uv_buf_t {
  pub base: *mut c_char,
  pub len: usize,
}

pub type uv_timer_cb = unsafe extern "C" fn(*mut uv_timer_t);
pub type uv_idle_cb = unsafe extern "C" fn(*mut uv_idle_t);
pub type uv_prepare_cb = unsafe extern "C" fn(*mut uv_prepare_t);
pub type uv_check_cb = unsafe extern "C" fn(*mut uv_check_t);
pub type uv_close_cb = unsafe extern "C" fn(*mut uv_handle_t);
pub type uv_write_cb = unsafe extern "C" fn(*mut uv_write_t, i32);
pub type uv_alloc_cb =
  unsafe extern "C" fn(*mut uv_handle_t, usize, *mut uv_buf_t);
pub type uv_read_cb =
  unsafe extern "C" fn(*mut uv_stream_t, isize, *const uv_buf_t);
pub type uv_connection_cb = unsafe extern "C" fn(*mut uv_stream_t, i32);
pub type uv_connect_cb = unsafe extern "C" fn(*mut uv_connect_t, i32);
pub type uv_shutdown_cb = unsafe extern "C" fn(*mut uv_shutdown_t, i32);

pub type UvHandle = uv_handle_t;
pub type UvLoop = uv_loop_t;
pub type UvStream = uv_stream_t;
pub type UvTcp = uv_tcp_t;
pub type UvTty = uv_tty_t;
pub type UvWrite = uv_write_t;
pub type UvBuf = uv_buf_t;
pub type UvConnect = uv_connect_t;
pub type UvShutdown = uv_shutdown_t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TimerKey {
  deadline_ms: u64,
  id: u64,
}

pub(crate) struct UvLoopInner {
  timers: RefCell<BTreeSet<TimerKey>>,
  next_timer_id: Cell<u64>,
  timer_handles: RefCell<HashMap<u64, *mut uv_timer_t>>,
  idle_handles: RefCell<Vec<*mut uv_idle_t>>,
  prepare_handles: RefCell<Vec<*mut uv_prepare_t>>,
  check_handles: RefCell<Vec<*mut uv_check_t>>,
  tcp_handles: RefCell<Vec<*mut uv_tcp_t>>,
  tty_handles: RefCell<Vec<*mut uv_tty_t>>,
  waker: RefCell<Option<Waker>>,
  closing_handles: RefCell<VecDeque<(*mut uv_handle_t, Option<uv_close_cb>)>>,
  time_origin: Instant,
}

impl UvLoopInner {
  fn new() -> Self {
    Self {
      timers: RefCell::new(BTreeSet::new()),
      next_timer_id: Cell::new(1),
      timer_handles: RefCell::new(HashMap::with_capacity(16)),
      idle_handles: RefCell::new(Vec::with_capacity(8)),
      prepare_handles: RefCell::new(Vec::with_capacity(8)),
      check_handles: RefCell::new(Vec::with_capacity(8)),
      tcp_handles: RefCell::new(Vec::with_capacity(8)),
      tty_handles: RefCell::new(Vec::with_capacity(4)),
      waker: RefCell::new(None),
      closing_handles: RefCell::new(VecDeque::with_capacity(16)),
      time_origin: Instant::now(),
    }
  }

  pub(crate) fn set_waker(&self, waker: &Waker) {
    let mut slot = self.waker.borrow_mut();
    match slot.as_ref() {
      Some(existing) if existing.will_wake(waker) => {}
      _ => *slot = Some(waker.clone()),
    }
  }

  #[inline]
  fn alloc_timer_id(&self) -> u64 {
    let id = self.next_timer_id.get();
    self.next_timer_id.set(id + 1);
    id
  }

  #[inline]
  fn now_ms(&self) -> u64 {
    Instant::now().duration_since(self.time_origin).as_millis() as u64
  }

  pub(crate) fn has_alive_handles(&self) -> bool {
    for (_, handle_ptr) in self.timer_handles.borrow().iter() {
      // SAFETY: Handle pointers in timer_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    for handle_ptr in self.idle_handles.borrow().iter() {
      // SAFETY: Handle pointers in idle_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    for handle_ptr in self.prepare_handles.borrow().iter() {
      // SAFETY: Handle pointers in prepare_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    for handle_ptr in self.check_handles.borrow().iter() {
      // SAFETY: Handle pointers in check_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    for handle_ptr in self.tcp_handles.borrow().iter() {
      // SAFETY: Handle pointers in tcp_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    for handle_ptr in self.tty_handles.borrow().iter() {
      // SAFETY: Handle pointers in tty_handles are kept valid by the C caller until uv_close.
      let handle = unsafe { &**handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && handle.flags & UV_HANDLE_REF != 0
      {
        return true;
      }
    }
    if !self.closing_handles.borrow().is_empty() {
      return true;
    }
    false
  }

  /// ### Safety
  /// All timer handle pointers stored in `timer_handles` must be valid.
  pub(crate) unsafe fn run_timers(&self) {
    let now = self.now_ms();
    let mut expired = Vec::new();
    {
      let timers = self.timers.borrow();
      for key in timers.iter() {
        if key.deadline_ms > now {
          break;
        }
        expired.push(*key);
      }
    }

    for key in expired {
      self.timers.borrow_mut().remove(&key);
      let handle_ptr = match self.timer_handles.borrow().get(&key.id).copied() {
        Some(h) => h,
        None => continue,
      };
      // SAFETY: handle_ptr comes from timer_handles; caller guarantees validity.
      let handle = unsafe { &mut *handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE == 0 {
        self.timer_handles.borrow_mut().remove(&key.id);
        continue;
      }
      let cb = handle.cb;
      let repeat = handle.repeat;

      if repeat > 0 {
        let new_deadline = now + repeat;
        let new_key = TimerKey {
          deadline_ms: new_deadline,
          id: key.id,
        };
        handle.internal_deadline = new_deadline;
        self.timers.borrow_mut().insert(new_key);
      } else {
        handle.flags &= !UV_HANDLE_ACTIVE;
        self.timer_handles.borrow_mut().remove(&key.id);
      }

      if let Some(cb) = cb {
        // SAFETY: handle_ptr is valid; cb was set by the C caller via uv_timer_start.
        unsafe { cb(handle_ptr) };
      }
    }
  }

  /// ### Safety
  /// All idle handle pointers stored in `idle_handles` must be valid.
  pub(crate) unsafe fn run_idle(&self) {
    let mut i = 0;
    loop {
      let handle_ptr = {
        let handles = self.idle_handles.borrow();
        if i >= handles.len() {
          break;
        }
        handles[i]
      };
      i += 1;
      // SAFETY: handle_ptr comes from idle_handles; caller guarantees validity.
      let handle = unsafe { &*handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && let Some(cb) = handle.cb
      {
        // SAFETY: Callback set by C caller via uv_idle_start; handle_ptr is valid.
        unsafe { cb(handle_ptr) };
      }
    }
  }

  /// ### Safety
  /// All prepare handle pointers stored in `prepare_handles` must be valid.
  pub(crate) unsafe fn run_prepare(&self) {
    let mut i = 0;
    loop {
      let handle_ptr = {
        let handles = self.prepare_handles.borrow();
        if i >= handles.len() {
          break;
        }
        handles[i]
      };
      i += 1;
      // SAFETY: handle_ptr comes from prepare_handles; caller guarantees validity.
      let handle = unsafe { &*handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && let Some(cb) = handle.cb
      {
        // SAFETY: Callback set by C caller via uv_prepare_start; handle_ptr is valid.
        unsafe { cb(handle_ptr) };
      }
    }
  }

  /// ### Safety
  /// All check handle pointers stored in `check_handles` must be valid.
  pub(crate) unsafe fn run_check(&self) {
    let mut i = 0;
    loop {
      let handle_ptr = {
        let handles = self.check_handles.borrow();
        if i >= handles.len() {
          break;
        }
        handles[i]
      };
      i += 1;
      // SAFETY: handle_ptr comes from check_handles; caller guarantees validity.
      let handle = unsafe { &*handle_ptr };
      if handle.flags & UV_HANDLE_ACTIVE != 0
        && let Some(cb) = handle.cb
      {
        // SAFETY: Callback set by C caller via uv_check_start; handle_ptr is valid.
        unsafe { cb(handle_ptr) };
      }
    }
  }

  /// ### Safety
  /// All handle pointers in `closing_handles` must be valid.
  pub(crate) unsafe fn run_close(&self) {
    let mut closing = self.closing_handles.borrow_mut();
    let snapshot: Vec<_> = closing.drain(..).collect();
    drop(closing);
    for (handle_ptr, cb) in snapshot {
      if let Some(cb) = cb {
        // SAFETY: handle_ptr is valid; cb was registered by C caller via uv_close.
        unsafe { cb(handle_ptr) };
      }
    }
  }

  /// Poll all TCP handles for I/O readiness and fire callbacks.
  ///
  /// Uses direct polling via tokio's `poll_accept`/`try_read`/`try_write`.
  /// No spawned tasks, no channels -- zero allocation in the hot path.
  ///
  /// Multiple passes: after callbacks fire they may produce new data
  /// (e.g. HTTP2 frame processing triggers writes which complete
  /// immediately). Re-poll up to 16 times to batch I/O within a
  /// single event loop tick.
  ///
  /// # Safety
  /// All TCP handle pointers in `tcp_handles` must be valid.
  pub(crate) unsafe fn run_io(&self) -> bool {
    let noop = Waker::noop();
    let waker_ref = self.waker.borrow();
    let waker = waker_ref.as_ref().unwrap_or(noop);
    let mut cx = Context::from_waker(waker);

    let mut did_any_work = false;

    for _pass in 0..16 {
      let mut any_work = false;

      let mut i = 0;
      loop {
        let tcp_ptr = {
          let handles = self.tcp_handles.borrow();
          if i >= handles.len() {
            break;
          }
          handles[i]
        };
        i += 1;
        // SAFETY: tcp_ptr comes from tcp_handles; caller guarantees validity.
        let tcp = unsafe { &mut *tcp_ptr };
        if tcp.flags & UV_HANDLE_ACTIVE == 0 {
          continue;
        }

        // 1. Poll pending connect
        if let Some(ref mut pending) = tcp.internal_connect
          && let Poll::Ready(result) = pending.future.as_mut().poll(&mut cx)
        {
          let req = pending.req;
          let cb = pending.cb;
          let status = match result {
            Ok(stream) => {
              if tcp.internal_nodelay {
                stream.set_nodelay(true).ok();
              }
              tcp.internal_stream = Some(stream);
              0
            }
            Err(_) => UV_ECONNREFUSED,
          };
          tcp.internal_connect = None;
          // SAFETY: req pointer was provided by the C caller and remains valid until callback.
          unsafe {
            (*req).handle = tcp_ptr as *mut uv_stream_t;
          }
          if let Some(cb) = cb {
            // SAFETY: Callback and req pointer validated above; set by C caller via uv_tcp_connect.
            unsafe { cb(req, status) };
          }
        }

        // 2. Poll listener for new connections
        if let Some(ref listener) = tcp.internal_listener
          && tcp.internal_connection_cb.is_some()
        {
          while let Poll::Ready(Ok((stream, _))) = listener.poll_accept(&mut cx)
          {
            tcp.internal_backlog.push_back(stream);
            any_work = true;
          }
          while !tcp.internal_backlog.is_empty() {
            if let Some(cb) = tcp.internal_connection_cb {
              // SAFETY: tcp_ptr is valid; cb set by C caller via uv_listen.
              unsafe { cb(tcp_ptr as *mut uv_stream_t, 0) };
            }
            // If uv_accept wasn't called in the callback, stop
            // to avoid an infinite loop.
            if !tcp.internal_backlog.is_empty() {
              break;
            }
          }
        }

        // 3. Poll readable stream
        if tcp.internal_reading && tcp.internal_stream.is_some() {
          let alloc_cb = tcp.internal_alloc_cb;
          let read_cb = tcp.internal_read_cb;
          if let (Some(alloc_cb), Some(read_cb)) = (alloc_cb, read_cb) {
            // Register interest so tokio's reactor wakes us.
            let _ = tcp
              .internal_stream
              .as_ref()
              .unwrap()
              .poll_read_ready(&mut cx);

            loop {
              // Re-check after each callback: the callback may have
              // called uv_close or uv_read_stop.
              if !tcp.internal_reading || tcp.internal_stream.is_none() {
                break;
              }
              let mut buf = uv_buf_t {
                base: std::ptr::null_mut(),
                len: 0,
              };
              // SAFETY: alloc_cb set by C caller via uv_read_start; tcp_ptr is valid.
              unsafe {
                alloc_cb(tcp_ptr as *mut uv_handle_t, 65536, &mut buf);
              }
              if buf.base.is_null() || buf.len == 0 {
                break;
              }
              // SAFETY: alloc_cb guarantees buf.base is valid for buf.len bytes.
              let slice = unsafe {
                std::slice::from_raw_parts_mut(buf.base.cast::<u8>(), buf.len)
              };
              match tcp.internal_stream.as_ref().unwrap().try_read(slice) {
                Ok(0) => {
                  // SAFETY: read_cb set by C caller via uv_read_start; tcp_ptr and buf are valid.
                  unsafe {
                    read_cb(tcp_ptr as *mut uv_stream_t, UV_EOF as isize, &buf)
                  };
                  tcp.internal_reading = false;
                  break;
                }
                Ok(n) => {
                  any_work = true;
                  // SAFETY: read_cb set by C caller via uv_read_start; tcp_ptr and buf are valid.
                  unsafe {
                    read_cb(tcp_ptr as *mut uv_stream_t, n as isize, &buf)
                  };
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                  break;
                }
                Err(_) => {
                  // SAFETY: read_cb set by C caller via uv_read_start; tcp_ptr and buf are valid.
                  unsafe {
                    read_cb(tcp_ptr as *mut uv_stream_t, UV_EOF as isize, &buf)
                  };
                  tcp.internal_reading = false;
                  break;
                }
              }
            }
          }
        }

        // 4. Drain write queue in order
        if !tcp.internal_write_queue.is_empty() && tcp.internal_stream.is_some()
        {
          let stream = tcp.internal_stream.as_ref().unwrap();
          let _ = stream.poll_write_ready(&mut cx);

          while let Some(pw) = tcp.internal_write_queue.front_mut() {
            let mut done = false;
            let mut error = false;
            loop {
              if pw.offset >= pw.data.len() {
                done = true;
                break;
              }
              match stream.try_write(&pw.data[pw.offset..]) {
                Ok(n) => pw.offset += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                  break;
                }
                Err(_) => {
                  error = true;
                  break;
                }
              }
            }
            if done {
              let pw = tcp.internal_write_queue.pop_front().unwrap();
              if let Some(cb) = pw.cb {
                // SAFETY: Write cb and req set by C caller via uv_write; req is valid until callback.
                unsafe { cb(pw.req, 0) };
              }
            } else if error {
              let pw = tcp.internal_write_queue.pop_front().unwrap();
              if let Some(cb) = pw.cb {
                // SAFETY: Write cb and req set by C caller via uv_write; req is valid until callback.
                unsafe { cb(pw.req, UV_EPIPE) };
              }
            } else {
              break; // WouldBlock -- retry next tick
            }
          }
        }
      } // end per-TCP-handle loop

      // --- TTY handle I/O ---
      #[cfg(unix)]
      {
        use std::os::unix::io::AsRawFd;
        let mut j = 0;
        loop {
          let tty_ptr = {
            let handles = self.tty_handles.borrow();
            if j >= handles.len() {
              break;
            }
            handles[j]
          };
          j += 1;
          // SAFETY: tty_ptr comes from tty_handles; caller guarantees validity.
          let tty = unsafe { &mut *tty_ptr };
          if tty.flags & UV_HANDLE_ACTIVE == 0 {
            continue;
          }

          // Read
          if tty.internal_reading && tty.internal_async_fd.is_some() {
            let alloc_cb = tty.internal_alloc_cb;
            let read_cb = tty.internal_read_cb;
            if let (Some(alloc_cb), Some(read_cb)) = (alloc_cb, read_cb) {
              let async_fd = tty.internal_async_fd.as_ref().unwrap();
              let _ = async_fd.poll_read_ready(&mut cx);

              loop {
                if !tty.internal_reading || tty.internal_async_fd.is_none() {
                  break;
                }
                let mut buf = uv_buf_t {
                  base: std::ptr::null_mut(),
                  len: 0,
                };
                unsafe {
                  alloc_cb(tty_ptr as *mut uv_handle_t, 65536, &mut buf);
                }
                if buf.base.is_null() || buf.len == 0 {
                  break;
                }
                let async_fd = tty.internal_async_fd.as_ref().unwrap();
                match async_fd.try_io(tokio::io::Interest::READABLE, |fd| {
                  let n = unsafe {
                    libc::read(fd.as_raw_fd(), buf.base as *mut c_void, buf.len)
                  };
                  if n < 0 {
                    Err(std::io::Error::last_os_error())
                  } else {
                    Ok(n as usize)
                  }
                }) {
                  Ok(0) => {
                    unsafe {
                      read_cb(
                        tty_ptr as *mut uv_stream_t,
                        UV_EOF as isize,
                        &buf,
                      )
                    };
                    tty.internal_reading = false;
                    break;
                  }
                  Ok(n) => {
                    any_work = true;
                    unsafe {
                      read_cb(tty_ptr as *mut uv_stream_t, n as isize, &buf)
                    };
                  }
                  Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                  }
                  Err(_) => {
                    unsafe {
                      read_cb(
                        tty_ptr as *mut uv_stream_t,
                        UV_EOF as isize,
                        &buf,
                      )
                    };
                    tty.internal_reading = false;
                    break;
                  }
                }
              }
            }
          }

          // Write
          if !tty.internal_write_queue.is_empty() {
            if tty.flags & UV_HANDLE_BLOCKING_WRITES != 0 {
              // Blocking writes (fallback for master PTYs)
              while let Some(pw) = tty.internal_write_queue.front_mut() {
                let mut done = false;
                let mut error = false;
                loop {
                  if pw.offset >= pw.data.len() {
                    done = true;
                    break;
                  }
                  let n = unsafe {
                    libc::write(
                      tty.internal_fd,
                      pw.data[pw.offset..].as_ptr() as *const c_void,
                      pw.data.len() - pw.offset,
                    )
                  };
                  if n > 0 {
                    pw.offset += n as usize;
                  } else {
                    error = true;
                    break;
                  }
                }
                if done {
                  let pw = tty.internal_write_queue.pop_front().unwrap();
                  if let Some(cb) = pw.cb {
                    unsafe { cb(pw.req, 0) };
                  }
                } else if error {
                  let pw = tty.internal_write_queue.pop_front().unwrap();
                  if let Some(cb) = pw.cb {
                    unsafe { cb(pw.req, UV_EPIPE) };
                  }
                } else {
                  break;
                }
              }
            } else if tty.internal_async_fd.is_some() {
              let async_fd = tty.internal_async_fd.as_ref().unwrap();
              let _ = async_fd.poll_write_ready(&mut cx);

              while let Some(pw) = tty.internal_write_queue.front_mut() {
                let mut done = false;
                let mut error = false;
                loop {
                  if pw.offset >= pw.data.len() {
                    done = true;
                    break;
                  }
                  let async_fd = tty.internal_async_fd.as_ref().unwrap();
                  match async_fd.try_io(tokio::io::Interest::WRITABLE, |fd| {
                    let n = unsafe {
                      libc::write(
                        fd.as_raw_fd(),
                        pw.data[pw.offset..].as_ptr() as *const c_void,
                        pw.data.len() - pw.offset,
                      )
                    };
                    if n < 0 {
                      Err(std::io::Error::last_os_error())
                    } else {
                      Ok(n as usize)
                    }
                  }) {
                    Ok(n) => pw.offset += n,
                    Err(ref e)
                      if e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                      break;
                    }
                    Err(_) => {
                      error = true;
                      break;
                    }
                  }
                }
                if done {
                  let pw = tty.internal_write_queue.pop_front().unwrap();
                  if let Some(cb) = pw.cb {
                    unsafe { cb(pw.req, 0) };
                  }
                } else if error {
                  let pw = tty.internal_write_queue.pop_front().unwrap();
                  if let Some(cb) = pw.cb {
                    unsafe { cb(pw.req, UV_EPIPE) };
                  }
                } else {
                  break;
                }
              }
            }
          }
        } // end per-TTY-handle loop
      }

      if !any_work {
        break;
      }
      did_any_work = true;
    } // end multi-pass loop

    did_any_work
  }

  /// ### Safety
  /// `handle` must be a valid pointer to an initialized `uv_timer_t`.
  unsafe fn stop_timer(&self, handle: *mut uv_timer_t) {
    // SAFETY: Caller guarantees handle is valid and initialized.
    let handle_ref = unsafe { &mut *handle };
    let id = handle_ref.internal_id;
    if id != 0 {
      let key = TimerKey {
        deadline_ms: handle_ref.internal_deadline,
        id,
      };
      self.timers.borrow_mut().remove(&key);
      self.timer_handles.borrow_mut().remove(&id);
    }
    handle_ref.flags &= !UV_HANDLE_ACTIVE;
  }

  fn stop_idle(&self, handle: *mut uv_idle_t) {
    self
      .idle_handles
      .borrow_mut()
      .retain(|&h| !std::ptr::eq(h, handle));
    // SAFETY: Caller guarantees handle is valid and initialized.
    unsafe {
      (*handle).flags &= !UV_HANDLE_ACTIVE;
    }
  }

  fn stop_prepare(&self, handle: *mut uv_prepare_t) {
    self
      .prepare_handles
      .borrow_mut()
      .retain(|&h| !std::ptr::eq(h, handle));
    // SAFETY: Caller guarantees handle is valid and initialized.
    unsafe {
      (*handle).flags &= !UV_HANDLE_ACTIVE;
    }
  }

  fn stop_check(&self, handle: *mut uv_check_t) {
    self
      .check_handles
      .borrow_mut()
      .retain(|&h| !std::ptr::eq(h, handle));
    // SAFETY: Caller guarantees handle is valid and initialized.
    unsafe {
      (*handle).flags &= !UV_HANDLE_ACTIVE;
    }
  }

  fn stop_tcp(&self, handle: *mut uv_tcp_t) {
    self
      .tcp_handles
      .borrow_mut()
      .retain(|&h| !std::ptr::eq(h, handle));
    // SAFETY: Caller guarantees handle is valid and initialized.
    unsafe {
      let tcp = &mut *handle;
      tcp.internal_reading = false;
      tcp.internal_alloc_cb = None;
      tcp.internal_read_cb = None;
      tcp.internal_connection_cb = None;
      tcp.internal_connect = None;
      tcp.internal_write_queue.clear();
      tcp.internal_stream = None;
      tcp.internal_listener = None;
      tcp.internal_backlog.clear();
      tcp.flags &= !UV_HANDLE_ACTIVE;
    }
  }

  fn stop_tty(&self, handle: *mut uv_tty_t) {
    self
      .tty_handles
      .borrow_mut()
      .retain(|&h| !std::ptr::eq(h, handle));
    // SAFETY: Caller guarantees handle is valid and initialized.
    unsafe {
      let tty = &mut *handle;
      #[cfg(unix)]
      {
        // Restore original termios if saved
        if let Some(ref orig) = tty.internal_orig_termios {
          libc::tcsetattr(tty.internal_fd, libc::TCSANOW, orig);
        }
        // Clear static reset state if this fd matches
        if TTY_RESET_FD == tty.internal_fd {
          TTY_RESET_FD = -1;
        }
        // Drop the AsyncFd to deregister from tokio reactor
        tty.internal_async_fd = None;
      }
      #[cfg(windows)]
      {
        if !tty.internal_handle.is_null() {
          windows_sys::Win32::Foundation::CloseHandle(
            tty.internal_handle as isize,
          );
          tty.internal_handle = std::ptr::null_mut();
        }
      }
      tty.internal_reading = false;
      tty.internal_alloc_cb = None;
      tty.internal_read_cb = None;
      tty.internal_write_queue.clear();
      tty.flags &= !UV_HANDLE_ACTIVE;
    }
  }
}

/// ### Safety
/// `loop_` must be a valid pointer to a `uv_loop_t` previously initialized by `uv_loop_init`.
#[inline]
unsafe fn get_inner(loop_: *mut uv_loop_t) -> &'static UvLoopInner {
  // SAFETY: Caller guarantees loop_ is valid and was initialized by uv_loop_init.
  unsafe { &*((*loop_).internal as *const UvLoopInner) }
}

/// ### Safety
/// `loop_` must be a valid pointer to a `uv_loop_t` previously initialized by `uv_loop_init`.
pub unsafe fn uv_loop_get_inner_ptr(
  loop_: *const uv_loop_t,
) -> *const std::ffi::c_void {
  // SAFETY: Caller guarantees loop_ is valid and was initialized by uv_loop_init.
  unsafe { (*loop_).internal as *const std::ffi::c_void }
}

/// ### Safety
/// `loop_` must be a valid, non-null pointer to an uninitialized `uv_loop_t`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_loop_init(loop_: *mut uv_loop_t) -> c_int {
  let inner = Box::new(UvLoopInner::new());
  // SAFETY: Caller guarantees loop_ is a valid, writable pointer.
  unsafe {
    (*loop_).internal = Box::into_raw(inner) as *mut c_void;
    (*loop_).data = std::ptr::null_mut();
    (*loop_).stop_flag = Cell::new(false);
  }
  0
}

/// ### Safety
/// `loop_` must be a valid pointer to a `uv_loop_t` initialized by `uv_loop_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_loop_close(loop_: *mut uv_loop_t) -> c_int {
  // SAFETY: Caller guarantees loop_ was initialized by uv_loop_init.
  unsafe {
    let internal = (*loop_).internal;
    if !internal.is_null() {
      drop(Box::from_raw(internal as *mut UvLoopInner));
      (*loop_).internal = std::ptr::null_mut();
    }
  }
  0
}

/// ### Safety
/// `loop_` must be a valid pointer to a `uv_loop_t` initialized by `uv_loop_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_now(loop_: *mut uv_loop_t) -> u64 {
  // SAFETY: Caller guarantees loop_ was initialized by uv_loop_init.
  let inner = unsafe { get_inner(loop_) };
  inner.now_ms()
}

/// ### Safety
/// `_loop_` must be a valid pointer to a `uv_loop_t` initialized by `uv_loop_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_update_time(_loop_: *mut uv_loop_t) {}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `handle` must be a valid, writable pointer.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_init(
  loop_: *mut uv_loop_t,
  handle: *mut uv_timer_t,
) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid.
  unsafe {
    (*handle).r#type = uv_handle_type::UV_TIMER;
    (*handle).loop_ = loop_;
    (*handle).data = std::ptr::null_mut();
    (*handle).flags = UV_HANDLE_REF;
    (*handle).internal_id = 0;
    (*handle).internal_deadline = 0;
    (*handle).cb = None;
    (*handle).timeout = 0;
    (*handle).repeat = 0;
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_timer_t` initialized by `uv_timer_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_start(
  handle: *mut uv_timer_t,
  cb: uv_timer_cb,
  timeout: u64,
  repeat: u64,
) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_timer_init.
  unsafe {
    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);

    if (*handle).flags & UV_HANDLE_ACTIVE != 0 {
      inner.stop_timer(handle);
    }

    let id = inner.alloc_timer_id();
    let deadline = inner.now_ms() + timeout;

    (*handle).cb = Some(cb);
    (*handle).timeout = timeout;
    (*handle).repeat = repeat;
    (*handle).internal_id = id;
    (*handle).internal_deadline = deadline;
    (*handle).flags |= UV_HANDLE_ACTIVE;

    let key = TimerKey {
      deadline_ms: deadline,
      id,
    };
    inner.timers.borrow_mut().insert(key);
    inner.timer_handles.borrow_mut().insert(id, handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_timer_t` initialized by `uv_timer_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_stop(handle: *mut uv_timer_t) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_timer_init.
  unsafe {
    let loop_ = (*handle).loop_;
    if loop_.is_null() || (*loop_).internal.is_null() {
      (*handle).flags &= !UV_HANDLE_ACTIVE;
      return 0;
    }
    let inner = get_inner(loop_);
    inner.stop_timer(handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_timer_t` initialized by `uv_timer_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_again(handle: *mut uv_timer_t) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_timer_init.
  unsafe {
    let repeat = (*handle).repeat;
    if repeat == 0 {
      return UV_EINVAL;
    }
    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);

    inner.stop_timer(handle);

    let id = inner.alloc_timer_id();
    let deadline = inner.now_ms() + repeat;

    (*handle).internal_id = id;
    (*handle).internal_deadline = deadline;
    (*handle).flags |= UV_HANDLE_ACTIVE;

    let key = TimerKey {
      deadline_ms: deadline,
      id,
    };
    inner.timers.borrow_mut().insert(key);
    inner.timer_handles.borrow_mut().insert(id, handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_timer_t` initialized by `uv_timer_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_get_repeat(handle: *const uv_timer_t) -> u64 {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe { (*handle).repeat }
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_timer_t` initialized by `uv_timer_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_timer_set_repeat(
  handle: *mut uv_timer_t,
  repeat: u64,
) {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    (*handle).repeat = repeat;
  }
}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `handle` must be a valid, writable pointer.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_idle_init(
  loop_: *mut uv_loop_t,
  handle: *mut uv_idle_t,
) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid.
  unsafe {
    (*handle).r#type = uv_handle_type::UV_IDLE;
    (*handle).loop_ = loop_;
    (*handle).data = std::ptr::null_mut();
    (*handle).flags = UV_HANDLE_REF;
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_idle_t` initialized by `uv_idle_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_idle_start(
  handle: *mut uv_idle_t,
  cb: uv_idle_cb,
) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_idle_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE != 0 {
      (*handle).cb = Some(cb);
      return 0;
    }
    (*handle).cb = Some(cb);
    (*handle).flags |= UV_HANDLE_ACTIVE;

    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.idle_handles.borrow_mut().push(handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_idle_t` initialized by `uv_idle_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_idle_stop(handle: *mut uv_idle_t) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_idle_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE == 0 {
      return 0;
    }
    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.stop_idle(handle);
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `handle` must be a valid, writable pointer.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_prepare_init(
  loop_: *mut uv_loop_t,
  handle: *mut uv_prepare_t,
) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid.
  unsafe {
    (*handle).r#type = uv_handle_type::UV_PREPARE;
    (*handle).loop_ = loop_;
    (*handle).data = std::ptr::null_mut();
    (*handle).flags = UV_HANDLE_REF;
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_prepare_t` initialized by `uv_prepare_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_prepare_start(
  handle: *mut uv_prepare_t,
  cb: uv_prepare_cb,
) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_prepare_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE != 0 {
      (*handle).cb = Some(cb);
      return 0;
    }
    (*handle).cb = Some(cb);
    (*handle).flags |= UV_HANDLE_ACTIVE;

    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.prepare_handles.borrow_mut().push(handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_prepare_t` initialized by `uv_prepare_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_prepare_stop(handle: *mut uv_prepare_t) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_prepare_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE == 0 {
      return 0;
    }
    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.stop_prepare(handle);
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `handle` must be a valid, writable pointer.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_check_init(
  loop_: *mut uv_loop_t,
  handle: *mut uv_check_t,
) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid.
  unsafe {
    (*handle).r#type = uv_handle_type::UV_CHECK;
    (*handle).loop_ = loop_;
    (*handle).data = std::ptr::null_mut();
    (*handle).flags = UV_HANDLE_REF;
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_check_t` initialized by `uv_check_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_check_start(
  handle: *mut uv_check_t,
  cb: uv_check_cb,
) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_check_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE != 0 {
      (*handle).cb = Some(cb);
      return 0;
    }
    (*handle).cb = Some(cb);
    (*handle).flags |= UV_HANDLE_ACTIVE;

    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.check_handles.borrow_mut().push(handle);
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to a `uv_check_t` initialized by `uv_check_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_check_stop(handle: *mut uv_check_t) -> c_int {
  // SAFETY: Caller guarantees handle was initialized by uv_check_init.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE == 0 {
      return 0;
    }
    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);
    inner.stop_check(handle);
    (*handle).cb = None;
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to any uv handle type (timer, idle, tcp, etc.) initialized
/// by the corresponding `uv_*_init` function. Must not be called twice on the same handle.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_close(
  handle: *mut uv_handle_t,
  close_cb: Option<uv_close_cb>,
) {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    (*handle).flags |= UV_HANDLE_CLOSING;
    (*handle).flags &= !UV_HANDLE_ACTIVE;

    let loop_ = (*handle).loop_;
    let inner = get_inner(loop_);

    match (*handle).r#type {
      uv_handle_type::UV_TIMER => {
        inner.stop_timer(handle as *mut uv_timer_t);
      }
      uv_handle_type::UV_IDLE => {
        inner.stop_idle(handle as *mut uv_idle_t);
      }
      uv_handle_type::UV_PREPARE => {
        inner.stop_prepare(handle as *mut uv_prepare_t);
      }
      uv_handle_type::UV_CHECK => {
        inner.stop_check(handle as *mut uv_check_t);
      }
      uv_handle_type::UV_TCP => {
        inner.stop_tcp(handle as *mut uv_tcp_t);
      }
      uv_handle_type::UV_TTY => {
        inner.stop_tty(handle as *mut uv_tty_t);
      }
      _ => {}
    }

    inner
      .closing_handles
      .borrow_mut()
      .push_back((handle, close_cb));
  }
}

/// ### Safety
/// `handle` must be a valid pointer to an initialized uv handle.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_ref(handle: *mut uv_handle_t) {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    (*handle).flags |= UV_HANDLE_REF;
  }
}

/// ### Safety
/// `handle` must be a valid pointer to an initialized uv handle.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_unref(handle: *mut uv_handle_t) {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    (*handle).flags &= !UV_HANDLE_REF;
  }
}

/// ### Safety
/// `handle` must be a valid pointer to an initialized uv handle.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_is_active(handle: *const uv_handle_t) -> c_int {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    if (*handle).flags & UV_HANDLE_ACTIVE != 0 {
      1
    } else {
      0
    }
  }
}

/// ### Safety
/// `handle` must be a valid pointer to an initialized uv handle.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_is_closing(handle: *const uv_handle_t) -> c_int {
  // SAFETY: Caller guarantees handle is valid and initialized.
  unsafe {
    if (*handle).flags & UV_HANDLE_CLOSING != 0 {
      1
    } else {
      0
    }
  }
}

/// ### Safety
/// `addr` must point to a valid `sockaddr_in` or `sockaddr_in6` with correct `sa_family`.
unsafe fn sockaddr_to_std(addr: *const c_void) -> Option<SocketAddr> {
  let sa = addr as *const libc::sockaddr;
  // SAFETY: Caller guarantees addr points to a valid sockaddr.
  let family = unsafe { (*sa).sa_family as i32 };
  if family == AF_INET {
    // SAFETY: Family is AF_INET so addr is a valid sockaddr_in.
    let sin = unsafe { &*(addr as *const sockaddr_in) };
    let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
    let port = u16::from_be(sin.sin_port);
    Some(SocketAddr::from((ip, port)))
  } else if family == AF_INET6 {
    // SAFETY: Family is AF_INET6 so addr is a valid sockaddr_in6.
    let sin6 = unsafe { &*(addr as *const sockaddr_in6) };
    let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
    let port = u16::from_be(sin6.sin6_port);
    Some(SocketAddr::from((ip, port)))
  } else {
    None
  }
}

/// ### Safety
/// `out` must be writable and large enough for `sockaddr_in` or `sockaddr_in6`.
/// `len` must be a valid, writable pointer.
unsafe fn std_to_sockaddr(addr: SocketAddr, out: *mut c_void, len: *mut c_int) {
  match addr {
    SocketAddr::V4(v4) => {
      let sin = out as *mut sockaddr_in;
      // SAFETY: Caller guarantees out is large enough for sockaddr_in.
      unsafe {
        std::ptr::write_bytes(sin, 0, 1);
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        {
          (*sin).sin_len = std::mem::size_of::<sockaddr_in>() as u8;
        }
        (*sin).sin_family = AF_INET as sa_family_t;
        (*sin).sin_port = v4.port().to_be();
        (*sin).sin_addr.s_addr = u32::from(*v4.ip()).to_be();
        *len = std::mem::size_of::<sockaddr_in>() as c_int;
      }
    }
    SocketAddr::V6(v6) => {
      let sin6 = out as *mut sockaddr_in6;
      // SAFETY: Caller guarantees out is large enough for sockaddr_in6.
      unsafe {
        std::ptr::write_bytes(sin6, 0, 1);
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        {
          (*sin6).sin6_len = std::mem::size_of::<sockaddr_in6>() as u8;
        }
        (*sin6).sin6_family = AF_INET6 as sa_family_t;
        (*sin6).sin6_port = v6.port().to_be();
        (*sin6).sin6_addr.s6_addr = v6.ip().octets();
        (*sin6).sin6_scope_id = v6.scope_id();
        *len = std::mem::size_of::<sockaddr_in6>() as c_int;
      }
    }
  }
}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `tcp` must be a valid, writable pointer.
pub unsafe fn uv_tcp_init(loop_: *mut uv_loop_t, tcp: *mut uv_tcp_t) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid.
  unsafe {
    use std::ptr::addr_of_mut;
    use std::ptr::write;
    write(addr_of_mut!((*tcp).r#type), uv_handle_type::UV_TCP);
    write(addr_of_mut!((*tcp).loop_), loop_);
    write(addr_of_mut!((*tcp).data), std::ptr::null_mut());
    write(addr_of_mut!((*tcp).flags), UV_HANDLE_REF);
    write(addr_of_mut!((*tcp).internal_fd), None);
    write(addr_of_mut!((*tcp).internal_bind_addr), None);
    write(addr_of_mut!((*tcp).internal_stream), None);
    write(addr_of_mut!((*tcp).internal_listener), None);
    write(addr_of_mut!((*tcp).internal_listener_addr), None);
    write(addr_of_mut!((*tcp).internal_nodelay), false);
    write(addr_of_mut!((*tcp).internal_alloc_cb), None);
    write(addr_of_mut!((*tcp).internal_read_cb), None);
    write(addr_of_mut!((*tcp).internal_reading), false);
    write(addr_of_mut!((*tcp).internal_connect), None);
    write(addr_of_mut!((*tcp).internal_write_queue), VecDeque::new());
    write(addr_of_mut!((*tcp).internal_connection_cb), None);
    write(addr_of_mut!((*tcp).internal_backlog), VecDeque::new());
  }
  0
}

/// ### Safety
/// `tcp` must be a valid pointer to a `uv_tcp_t` initialized by `uv_tcp_init`.
/// `fd` must be a valid, open file descriptor / socket.
pub unsafe fn uv_tcp_open(tcp: *mut uv_tcp_t, fd: c_int) -> c_int {
  // SAFETY: Caller guarantees tcp is initialized and fd is valid.
  unsafe {
    #[cfg(unix)]
    let std_stream = {
      use std::os::unix::io::FromRawFd;
      let s = std::net::TcpStream::from_raw_fd(fd);
      (*tcp).internal_fd = Some(fd);
      s
    };
    #[cfg(windows)]
    let std_stream = {
      use std::os::windows::io::FromRawSocket;
      let sock = fd as std::os::windows::io::RawSocket;
      let s = std::net::TcpStream::from_raw_socket(sock);
      (*tcp).internal_fd = Some(sock);
      s
    };
    std_stream.set_nonblocking(true).ok();
    match tokio::net::TcpStream::from_std(std_stream) {
      Ok(stream) => {
        if (*tcp).internal_nodelay {
          stream.set_nodelay(true).ok();
        }
        (*tcp).internal_stream = Some(stream);
        0
      }
      Err(_) => UV_EINVAL,
    }
  }
}

/// ### Safety
/// `tcp` must be initialized by `uv_tcp_init`. `addr` must point to a valid sockaddr.
pub unsafe fn uv_tcp_bind(
  tcp: *mut uv_tcp_t,
  addr: *const c_void,
  _addrlen: u32,
  _flags: u32,
) -> c_int {
  // SAFETY: Caller guarantees addr points to a valid sockaddr.
  let sock_addr = unsafe { sockaddr_to_std(addr) };
  match sock_addr {
    Some(sa) => {
      // SAFETY: Caller guarantees tcp is valid and initialized.
      unsafe { (*tcp).internal_bind_addr = Some(sa) };
      0
    }
    None => UV_EINVAL,
  }
}

/// ### Safety
/// `req` must be a valid, writable pointer. `tcp` must be initialized by `uv_tcp_init`.
/// `addr` must point to a valid sockaddr. `req` must remain valid until the connect callback fires.
pub unsafe fn uv_tcp_connect(
  req: *mut uv_connect_t,
  tcp: *mut uv_tcp_t,
  addr: *const c_void,
  cb: Option<uv_connect_cb>,
) -> c_int {
  // SAFETY: Caller guarantees addr points to a valid sockaddr.
  let sock_addr = unsafe { sockaddr_to_std(addr) };
  let sock_addr = match sock_addr {
    Some(sa) => sa,
    None => return UV_EINVAL,
  };

  // SAFETY: Caller guarantees req and tcp are valid.
  unsafe {
    (*req).handle = tcp as *mut uv_stream_t;
  }

  // SAFETY: tcp was initialized by uv_tcp_init which set loop_.
  let inner = unsafe { get_inner((*tcp).loop_) };

  // SAFETY: Caller guarantees tcp is valid and initialized.
  unsafe {
    (*tcp).flags |= UV_HANDLE_ACTIVE;
    let mut handles = inner.tcp_handles.borrow_mut();
    if !handles.iter().any(|&h| std::ptr::eq(h, tcp)) {
      handles.push(tcp);
    }

    (*tcp).internal_connect = Some(ConnectPending {
      future: Box::pin(tokio::net::TcpStream::connect(sock_addr)),
      req,
      cb,
    });
  }

  0
}

/// ### Safety
/// `tcp` must be a valid pointer to a `uv_tcp_t` initialized by `uv_tcp_init`.
pub unsafe fn uv_tcp_nodelay(tcp: *mut uv_tcp_t, enable: c_int) -> c_int {
  // SAFETY: Caller guarantees tcp is valid and initialized.
  unsafe {
    let enabled = enable != 0;
    (*tcp).internal_nodelay = enabled;
    if let Some(ref stream) = (*tcp).internal_stream
      && stream.set_nodelay(enabled).is_err()
    {
      return UV_EINVAL;
    }
  }
  0
}

/// ### Safety
/// `tcp` must be initialized by `uv_tcp_init`. `name` must be writable and large enough
/// for a sockaddr. `namelen` must be a valid, writable pointer.
pub unsafe fn uv_tcp_getpeername(
  tcp: *const uv_tcp_t,
  name: *mut c_void,
  namelen: *mut c_int,
) -> c_int {
  // SAFETY: Caller guarantees all pointers are valid.
  unsafe {
    if let Some(ref stream) = (*tcp).internal_stream {
      match stream.peer_addr() {
        Ok(addr) => {
          std_to_sockaddr(addr, name, namelen);
          0
        }
        Err(_) => UV_ENOTCONN,
      }
    } else {
      UV_ENOTCONN
    }
  }
}

/// ### Safety
/// `tcp` must be initialized by `uv_tcp_init`. `name` must be writable and large enough
/// for a sockaddr. `namelen` must be a valid, writable pointer.
pub unsafe fn uv_tcp_getsockname(
  tcp: *const uv_tcp_t,
  name: *mut c_void,
  namelen: *mut c_int,
) -> c_int {
  // SAFETY: Caller guarantees all pointers are valid.
  unsafe {
    if let Some(ref stream) = (*tcp).internal_stream {
      match stream.local_addr() {
        Ok(addr) => {
          std_to_sockaddr(addr, name, namelen);
          return 0;
        }
        Err(_) => return UV_EINVAL,
      }
    }
    if let Some(addr) = (*tcp).internal_listener_addr {
      std_to_sockaddr(addr, name, namelen);
      return 0;
    }
    if let Some(addr) = (*tcp).internal_bind_addr {
      std_to_sockaddr(addr, name, namelen);
      return 0;
    }
    UV_EINVAL
  }
}

/// ### Safety
/// `_tcp` must be a valid pointer to a `uv_tcp_t` initialized by `uv_tcp_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_tcp_keepalive(
  _tcp: *mut uv_tcp_t,
  _enable: c_int,
  _delay: c_uint,
) -> c_int {
  // Keepalive is a no-op: tokio's TcpStream doesn't expose SO_KEEPALIVE
  // configuration in a cross-platform way, and nghttp2 only uses this
  // as a best-effort hint.
  0
}

/// ### Safety
/// `_tcp` must be a valid pointer to a `uv_tcp_t` initialized by `uv_tcp_init`.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_tcp_simultaneous_accepts(
  _tcp: *mut uv_tcp_t,
  _enable: c_int,
) -> c_int {
  0 // no-op
}

/// ### Safety
/// `ip` must be a valid, null-terminated C string. `addr` must be a valid, writable pointer.
#[cfg_attr(feature = "uv_compat_export", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_ip4_addr(
  ip: *const c_char,
  port: c_int,
  addr: *mut sockaddr_in,
) -> c_int {
  // SAFETY: Caller guarantees ip is a valid C string and addr is writable.
  unsafe {
    let c_str = std::ffi::CStr::from_ptr(ip);
    let Ok(s) = c_str.to_str() else {
      return UV_EINVAL;
    };
    let Ok(ip_addr) = s.parse::<std::net::Ipv4Addr>() else {
      return UV_EINVAL;
    };
    std::ptr::write_bytes(addr, 0, 1);
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
      (*addr).sin_len = std::mem::size_of::<sockaddr_in>() as u8;
    }
    (*addr).sin_family = AF_INET as sa_family_t;
    (*addr).sin_port = (port as u16).to_be();
    (*addr).sin_addr.s_addr = u32::from(ip_addr).to_be();
    0
  }
}

/// ### Safety
/// `stream` must be a valid pointer to a `uv_tcp_t` (cast as `uv_stream_t`) initialized
/// by `uv_tcp_init`, with a bind address set via `uv_tcp_bind`.
pub unsafe fn uv_listen(
  stream: *mut uv_stream_t,
  _backlog: c_int,
  cb: Option<uv_connection_cb>,
) -> c_int {
  // SAFETY: Caller guarantees stream is a valid, initialized uv_tcp_t.
  unsafe {
    let tcp = stream as *mut uv_tcp_t;
    let tcp_ref = &mut *tcp;

    let bind_addr = tcp_ref
      .internal_bind_addr
      .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());

    let std_listener = match std::net::TcpListener::bind(bind_addr) {
      Ok(l) => l,
      Err(_) => return UV_EADDRINUSE,
    };
    std_listener.set_nonblocking(true).ok();
    let listener_addr = std_listener.local_addr().ok();
    let tokio_listener = match tokio::net::TcpListener::from_std(std_listener) {
      Ok(l) => l,
      Err(_) => return UV_EINVAL,
    };

    tcp_ref.internal_listener = Some(tokio_listener);
    tcp_ref.internal_listener_addr = listener_addr;
    tcp_ref.internal_connection_cb = cb;
    tcp_ref.flags |= UV_HANDLE_ACTIVE;

    let inner = get_inner(tcp_ref.loop_);
    let mut handles = inner.tcp_handles.borrow_mut();
    if !handles.iter().any(|&h| std::ptr::eq(h, tcp)) {
      handles.push(tcp);
    }
  }
  0
}

/// ### Safety
/// `server` must be a listening `uv_tcp_t`. `client` must be initialized by `uv_tcp_init`.
pub unsafe fn uv_accept(
  server: *mut uv_stream_t,
  client: *mut uv_stream_t,
) -> c_int {
  // SAFETY: Caller guarantees both pointers are valid, initialized uv_tcp_t handles.
  unsafe {
    let server_tcp = &mut *(server as *mut uv_tcp_t);
    let client_tcp = &mut *(client as *mut uv_tcp_t);

    match server_tcp.internal_backlog.pop_front() {
      Some(stream) => {
        if client_tcp.internal_nodelay {
          stream.set_nodelay(true).ok();
        }
        client_tcp.internal_stream = Some(stream);
        0
      }
      None => UV_EAGAIN,
    }
  }
}

/// ### Safety
/// `stream` must be a valid pointer to an initialized stream handle (`uv_tcp_t` or `uv_tty_t`).
pub unsafe fn uv_read_start(
  stream: *mut uv_stream_t,
  alloc_cb: Option<uv_alloc_cb>,
  read_cb: Option<uv_read_cb>,
) -> c_int {
  unsafe {
    match (*stream).r#type {
      uv_handle_type::UV_TCP => {
        let tcp = stream as *mut uv_tcp_t;
        let tcp_ref = &mut *tcp;
        tcp_ref.internal_alloc_cb = alloc_cb;
        tcp_ref.internal_read_cb = read_cb;
        tcp_ref.internal_reading = true;
        tcp_ref.flags |= UV_HANDLE_ACTIVE;

        let inner = get_inner(tcp_ref.loop_);
        let mut handles = inner.tcp_handles.borrow_mut();
        if !handles.iter().any(|&h| std::ptr::eq(h, tcp)) {
          handles.push(tcp);
        }
      }
      uv_handle_type::UV_TTY => {
        let tty = stream as *mut uv_tty_t;
        let tty_ref = &mut *tty;
        tty_ref.internal_alloc_cb = alloc_cb;
        tty_ref.internal_read_cb = read_cb;
        tty_ref.internal_reading = true;
        tty_ref.flags |= UV_HANDLE_ACTIVE;

        let inner = get_inner(tty_ref.loop_);
        let mut handles = inner.tty_handles.borrow_mut();
        if !handles.iter().any(|&h| std::ptr::eq(h, tty)) {
          handles.push(tty);
        }
      }
      _ => return UV_EINVAL,
    }
  }
  0
}

/// ### Safety
/// `stream` must be a valid pointer to an initialized stream handle (`uv_tcp_t` or `uv_tty_t`).
pub unsafe fn uv_read_stop(stream: *mut uv_stream_t) -> c_int {
  unsafe {
    match (*stream).r#type {
      uv_handle_type::UV_TCP => {
        let tcp = stream as *mut uv_tcp_t;
        let tcp_ref = &mut *tcp;
        tcp_ref.internal_reading = false;
        tcp_ref.internal_alloc_cb = None;
        tcp_ref.internal_read_cb = None;
        if tcp_ref.internal_connection_cb.is_none()
          && tcp_ref.internal_connect.is_none()
          && tcp_ref.internal_write_queue.is_empty()
        {
          tcp_ref.flags &= !UV_HANDLE_ACTIVE;
        }
      }
      uv_handle_type::UV_TTY => {
        let tty = stream as *mut uv_tty_t;
        let tty_ref = &mut *tty;
        tty_ref.internal_reading = false;
        tty_ref.internal_alloc_cb = None;
        tty_ref.internal_read_cb = None;
        if tty_ref.internal_write_queue.is_empty() {
          tty_ref.flags &= !UV_HANDLE_ACTIVE;
        }
      }
      _ => return UV_EINVAL,
    }
  }
  0
}

/// ### Safety
/// `handle` must be a valid pointer to an initialized stream handle (`uv_tcp_t` or `uv_tty_t`).
pub unsafe fn uv_try_write(handle: *mut uv_stream_t, data: &[u8]) -> i32 {
  unsafe {
    match (*handle).r#type {
      uv_handle_type::UV_TCP => {
        let tcp_ref = &mut *(handle as *mut uv_tcp_t);
        if !tcp_ref.internal_write_queue.is_empty() {
          return UV_EAGAIN;
        }
        let stream = match tcp_ref.internal_stream.as_ref() {
          Some(s) => s,
          None => return UV_EBADF,
        };
        match stream.try_write(data) {
          Ok(n) => n as i32,
          Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => UV_EAGAIN,
          Err(_) => UV_EPIPE,
        }
      }
      #[cfg(unix)]
      uv_handle_type::UV_TTY => {
        let tty_ref = &mut *(handle as *mut uv_tty_t);
        if !tty_ref.internal_write_queue.is_empty() {
          return UV_EAGAIN;
        }
        let n = libc::write(
          tty_ref.internal_fd,
          data.as_ptr() as *const c_void,
          data.len(),
        );
        if n >= 0 {
          n as i32
        } else {
          let err = std::io::Error::last_os_error();
          if err.kind() == std::io::ErrorKind::WouldBlock {
            UV_EAGAIN
          } else {
            UV_EPIPE
          }
        }
      }
      _ => UV_EINVAL,
    }
  }
}

/// ### Safety
/// `req` must be valid and remain so until the write callback fires. `handle` must be an
/// initialized stream handle. `bufs` must point to `nbufs` valid `uv_buf_t` entries.
pub unsafe fn uv_write(
  req: *mut uv_write_t,
  handle: *mut uv_stream_t,
  bufs: *const uv_buf_t,
  nbufs: u32,
  cb: Option<uv_write_cb>,
) -> c_int {
  // SAFETY: Caller guarantees all pointers are valid.
  unsafe {
    (*req).handle = handle;

    #[cfg(unix)]
    if (*handle).r#type == uv_handle_type::UV_TTY {
      let tty = handle as *mut uv_tty_t;
      let tty_ref = &mut *tty;
      let write_data = collect_bufs(bufs, nbufs);
      tty_ref.internal_write_queue.push_back(WritePending {
        req,
        data: write_data,
        offset: 0,
        cb,
      });
      let inner = get_inner(tty_ref.loop_);
      let mut handles = inner.tty_handles.borrow_mut();
      if !handles.iter().any(|&h| std::ptr::eq(h, tty)) {
        handles.push(tty);
      }
      tty_ref.flags |= UV_HANDLE_ACTIVE;
      return 0;
    }

    let tcp = handle as *mut uv_tcp_t;
    let tcp_ref = &mut *tcp;

    let stream = match tcp_ref.internal_stream.as_ref() {
      Some(s) => s,
      None => {
        if let Some(cb) = cb {
          cb(req, UV_ENOTCONN);
        }
        return 0;
      }
    };

    if !tcp_ref.internal_write_queue.is_empty() {
      let write_data = collect_bufs(bufs, nbufs);
      tcp_ref.internal_write_queue.push_back(WritePending {
        req,
        data: write_data,
        offset: 0,
        cb,
      });
      return 0;
    }

    if nbufs == 1 {
      let buf = &*bufs;
      if !buf.base.is_null() && buf.len > 0 {
        let data = std::slice::from_raw_parts(buf.base as *const u8, buf.len);
        let mut offset = 0;
        loop {
          match stream.try_write(&data[offset..]) {
            Ok(n) => {
              offset += n;
              if offset >= data.len() {
                if let Some(cb) = cb {
                  cb(req, 0);
                }
                return 0;
              }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
              tcp_ref.internal_write_queue.push_back(WritePending {
                req,
                data: data[offset..].to_vec(),
                offset: 0,
                cb,
              });
              return 0;
            }
            Err(_) => {
              if let Some(cb) = cb {
                cb(req, UV_EPIPE);
              }
              return 0;
            }
          }
        }
      }
      if let Some(cb) = cb {
        cb(req, 0);
      }
      return 0;
    }

    let iovecs: smallvec::SmallVec<[std::io::IoSlice<'_>; 8]> = (0..nbufs
      as usize)
      .filter_map(|i| {
        let buf = &*bufs.add(i);
        if buf.base.is_null() || buf.len == 0 {
          None
        } else {
          Some(std::io::IoSlice::new(std::slice::from_raw_parts(
            buf.base as *const u8,
            buf.len,
          )))
        }
      })
      .collect();

    let total_len: usize = iovecs.iter().map(|s| s.len()).sum();
    if total_len == 0 {
      if let Some(cb) = cb {
        cb(req, 0);
      }
      return 0;
    }

    match stream.try_write_vectored(&iovecs) {
      Ok(n) if n >= total_len => {
        if let Some(cb) = cb {
          cb(req, 0);
        }
        return 0;
      }
      Ok(n) => {
        let mut write_data = Vec::with_capacity(total_len - n);
        let mut skip = n;
        for iov in &iovecs {
          if skip >= iov.len() {
            skip -= iov.len();
          } else {
            write_data.extend_from_slice(&iov[skip..]);
            skip = 0;
          }
        }
        tcp_ref.internal_write_queue.push_back(WritePending {
          req,
          data: write_data,
          offset: 0,
          cb,
        });
      }
      Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        let write_data = collect_bufs(bufs, nbufs);
        tcp_ref.internal_write_queue.push_back(WritePending {
          req,
          data: write_data,
          offset: 0,
          cb,
        });
      }
      Err(_) => {
        if let Some(cb) = cb {
          cb(req, UV_EPIPE);
        }
      }
    }
  }
  0
}

/// ### Safety
/// `bufs` must point to `nbufs` valid `uv_buf_t` entries with valid `base` pointers.
unsafe fn collect_bufs(bufs: *const uv_buf_t, nbufs: u32) -> Vec<u8> {
  // SAFETY: Caller guarantees bufs points to nbufs valid entries.
  unsafe {
    let mut total = 0usize;
    for i in 0..nbufs as usize {
      let buf = &*bufs.add(i);
      if !buf.base.is_null() {
        total += buf.len;
      }
    }
    let mut data = Vec::with_capacity(total);
    for i in 0..nbufs as usize {
      let buf = &*bufs.add(i);
      if !buf.base.is_null() && buf.len > 0 {
        data.extend_from_slice(std::slice::from_raw_parts(
          buf.base as *const u8,
          buf.len,
        ));
      }
    }
    data
  }
}

/// ### Safety
/// `req` must be a valid, writable pointer. `stream` must be an initialized stream handle.
/// `req` must remain valid until the shutdown callback fires.
pub unsafe fn uv_shutdown(
  req: *mut uv_shutdown_t,
  stream: *mut uv_stream_t,
  cb: Option<uv_shutdown_cb>,
) -> c_int {
  // SAFETY: Caller guarantees all pointers are valid.
  unsafe {
    (*req).handle = stream;

    // TTY shutdown is a no-op (just fire callback with success)
    if (*stream).r#type == uv_handle_type::UV_TTY {
      if let Some(cb) = cb {
        cb(req, 0);
      }
      return 0;
    }

    let tcp = stream as *mut uv_tcp_t;
    let status = if let Some(ref stream) = (*tcp).internal_stream {
      #[cfg(unix)]
      {
        use std::os::unix::io::AsRawFd;
        let fd = stream.as_raw_fd();
        if libc::shutdown(fd, libc::SHUT_WR) == 0 {
          0
        } else {
          UV_ENOTCONN
        }
      }
      #[cfg(windows)]
      {
        use std::os::windows::io::AsRawSocket;
        let sock = stream.as_raw_socket();
        if win_sock::shutdown(sock as usize, win_sock::SD_SEND) == 0 {
          0
        } else {
          UV_ENOTCONN
        }
      }
    } else {
      UV_ENOTCONN
    };

    if let Some(cb) = cb {
      cb(req, status);
    }
  }
  0
}

// --- TTY functions ---

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `tty` must be a valid, writable pointer.
/// `fd` must be a valid file descriptor. `readable` indicates whether the fd is readable.
#[cfg(unix)]
pub unsafe fn uv_tty_init(
  loop_: *mut uv_loop_t,
  tty: *mut uv_tty_t,
  fd: c_int,
  _readable: c_int,
) -> c_int {
  use std::os::unix::io::FromRawFd;
  use std::os::unix::io::OwnedFd;
  use std::ptr::addr_of_mut;
  use std::ptr::write;

  unsafe {
    if libc::isatty(fd) == 0 {
      return UV_EINVAL;
    }

    let flags_val = libc::fcntl(fd, libc::F_GETFL);
    let access_mode = flags_val & libc::O_ACCMODE;

    let mut use_fd = fd;
    let mut blocking_writes = false;

    // Try to reopen PTY slave to avoid affecting other processes
    let mut tty_name = [0u8; 256];
    let rc =
      libc::ttyname_r(fd, tty_name.as_mut_ptr() as *mut c_char, tty_name.len());
    if rc == 0 {
      let open_mode = if access_mode == libc::O_RDONLY {
        libc::O_RDONLY
      } else if access_mode == libc::O_WRONLY {
        libc::O_WRONLY
      } else {
        libc::O_RDWR
      };
      let new_fd = libc::open(
        tty_name.as_ptr() as *const c_char,
        open_mode | libc::O_NOCTTY,
      );
      if new_fd >= 0 {
        // Successfully reopened - dup2 to replace original fd
        if libc::dup2(new_fd, fd) < 0 {
          libc::close(new_fd);
          // Fall through, use original fd
        } else {
          libc::close(new_fd);
          use_fd = fd;
        }
      } else if access_mode != libc::O_RDONLY {
        // Can't reopen for writing, use blocking writes
        blocking_writes = true;
      }
    }

    // Set non-blocking unless using blocking writes
    if !blocking_writes {
      let current = libc::fcntl(use_fd, libc::F_GETFL);
      libc::fcntl(use_fd, libc::F_SETFL, current | libc::O_NONBLOCK);
    }

    // Create AsyncFd for tokio reactor integration (dup to avoid double-close)
    let dup_fd = libc::dup(use_fd);
    if dup_fd < 0 {
      return UV_EIO;
    }
    if !blocking_writes {
      let current = libc::fcntl(dup_fd, libc::F_GETFL);
      libc::fcntl(dup_fd, libc::F_SETFL, current | libc::O_NONBLOCK);
    }
    let owned = OwnedFd::from_raw_fd(dup_fd);
    let async_fd = match tokio::io::unix::AsyncFd::new(owned) {
      Ok(afd) => Some(afd),
      Err(_) => {
        // If registering with tokio fails (e.g. blocking writes), that's ok
        if blocking_writes {
          None
        } else {
          return UV_EIO;
        }
      }
    };

    // Save original termios
    let mut orig_termios: libc::termios = std::mem::zeroed();
    let termios_saved = if libc::tcgetattr(use_fd, &mut orig_termios) == 0 {
      Some(orig_termios)
    } else {
      None
    };

    let mut handle_flags = UV_HANDLE_REF;
    if blocking_writes {
      handle_flags |= UV_HANDLE_BLOCKING_WRITES;
    }
    if access_mode != libc::O_WRONLY {
      handle_flags |= UV_HANDLE_TTY_READABLE;
    }

    write(addr_of_mut!((*tty).r#type), uv_handle_type::UV_TTY);
    write(addr_of_mut!((*tty).loop_), loop_);
    write(addr_of_mut!((*tty).data), std::ptr::null_mut());
    write(addr_of_mut!((*tty).flags), handle_flags);
    write(addr_of_mut!((*tty).internal_mode), UV_TTY_MODE_NORMAL);
    write(addr_of_mut!((*tty).internal_fd), use_fd);
    write(addr_of_mut!((*tty).internal_orig_termios), termios_saved);
    write(addr_of_mut!((*tty).internal_async_fd), async_fd);
    write(addr_of_mut!((*tty).internal_alloc_cb), None);
    write(addr_of_mut!((*tty).internal_read_cb), None);
    write(addr_of_mut!((*tty).internal_reading), false);
    write(addr_of_mut!((*tty).internal_write_queue), VecDeque::new());

    let inner = get_inner(loop_);
    inner.tty_handles.borrow_mut().push(tty);
  }
  0
}

/// ### Safety
/// `loop_` must be initialized by `uv_loop_init`. `tty` must be a valid, writable pointer.
/// `fd` must be a valid file descriptor. `readable` indicates whether the fd is readable.
#[cfg(windows)]
pub unsafe fn uv_tty_init(
  loop_: *mut uv_loop_t,
  tty: *mut uv_tty_t,
  fd: c_int,
  _readable: c_int,
) -> c_int {
  use std::ptr::addr_of_mut;
  use std::ptr::write;

  unsafe {
    let os_handle = libc::get_osfhandle(fd);
    if os_handle == -1 {
      return UV_EBADF;
    }

    let mut handle: *mut c_void = std::ptr::null_mut();
    let current_process =
      windows_sys::Win32::System::Threading::GetCurrentProcess();
    let ok = windows_sys::Win32::Foundation::DuplicateHandle(
      current_process,
      os_handle as isize,
      current_process,
      &mut handle as *mut _ as *mut isize,
      0,
      0,
      windows_sys::Win32::Foundation::DUPLICATE_SAME_ACCESS,
    );
    if ok == 0 {
      return UV_EIO;
    }

    // Determine if readable
    let mut num_events: u32 = 0;
    let is_readable =
      windows_sys::Win32::System::Console::GetNumberOfConsoleInputEvents(
        os_handle as isize,
        &mut num_events,
      ) != 0;

    let mut handle_flags = UV_HANDLE_REF;
    if is_readable {
      handle_flags |= UV_HANDLE_TTY_READABLE;
    }

    write(addr_of_mut!((*tty).r#type), uv_handle_type::UV_TTY);
    write(addr_of_mut!((*tty).loop_), loop_);
    write(addr_of_mut!((*tty).data), std::ptr::null_mut());
    write(addr_of_mut!((*tty).flags), handle_flags);
    write(addr_of_mut!((*tty).internal_mode), UV_TTY_MODE_NORMAL);
    write(addr_of_mut!((*tty).internal_handle), handle);
    write(addr_of_mut!((*tty).internal_alloc_cb), None);
    write(addr_of_mut!((*tty).internal_read_cb), None);
    write(addr_of_mut!((*tty).internal_reading), false);
    write(addr_of_mut!((*tty).internal_write_queue), VecDeque::new());

    let inner = get_inner(loop_);
    inner.tty_handles.borrow_mut().push(tty);
  }
  0
}

/// Set the TTY mode.
///
/// ### Safety
/// `handle` must be a valid pointer to a `uv_tty_t` initialized by `uv_tty_init`.
#[cfg(unix)]
pub unsafe extern "C" fn uv_tty_set_mode(
  handle: *mut uv_tty_t,
  mode: c_int,
) -> c_int {
  unsafe {
    let tty = &mut *handle;
    if mode == tty.internal_mode {
      return 0;
    }

    // When transitioning from normal to non-normal, save current termios
    // for uv_tty_reset_mode
    if tty.internal_mode == UV_TTY_MODE_NORMAL && mode != UV_TTY_MODE_NORMAL {
      let mut current: libc::termios = std::mem::zeroed();
      if libc::tcgetattr(tty.internal_fd, &mut current) == 0 {
        tty.internal_orig_termios = Some(current);
        // Save to global state for uv_tty_reset_mode (spinlock)
        while TTY_RESET_LOCK
          .compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
          )
          .is_err()
        {
          std::hint::spin_loop();
        }
        TTY_RESET_FD = tty.internal_fd;
        std::ptr::addr_of_mut!(TTY_RESET_TERMIOS)
          .cast::<libc::termios>()
          .write(current);
        TTY_RESET_LOCK.store(false, Ordering::Release);
      }
    }

    match mode {
      UV_TTY_MODE_NORMAL => {
        // Restore original termios
        if let Some(ref orig) = tty.internal_orig_termios
          && libc::tcsetattr(tty.internal_fd, libc::TCSADRAIN, orig) != 0
        {
          return UV_EIO;
        }
      }
      UV_TTY_MODE_RAW => {
        let mut raw: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(tty.internal_fd, &mut raw) != 0 {
          return UV_EIO;
        }
        raw.c_iflag &= !(libc::BRKINT
          | libc::ICRNL
          | libc::INPCK
          | libc::ISTRIP
          | libc::IXON);
        raw.c_oflag |= libc::ONLCR;
        raw.c_cflag |= libc::CS8;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(tty.internal_fd, libc::TCSADRAIN, &raw) != 0 {
          return UV_EIO;
        }
      }
      UV_TTY_MODE_IO => {
        let mut raw: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(tty.internal_fd, &mut raw) != 0 {
          return UV_EIO;
        }
        // cfmakeraw equivalent
        raw.c_iflag &= !(libc::BRKINT
          | libc::ICRNL
          | libc::INPCK
          | libc::ISTRIP
          | libc::IXON
          | libc::IMAXBEL
          | libc::IGNBRK
          | libc::IGNCR
          | libc::INLCR
          | libc::PARMRK);
        raw.c_oflag &= !libc::OPOST;
        raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
        raw.c_cflag |= libc::CS8;
        raw.c_lflag &= !(libc::ECHO
          | libc::ECHONL
          | libc::ICANON
          | libc::IEXTEN
          | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(tty.internal_fd, libc::TCSADRAIN, &raw) != 0 {
          return UV_EIO;
        }
      }
      _ => return UV_EINVAL,
    }

    tty.internal_mode = mode;
    0
  }
}

/// Set the TTY mode.
///
/// ### Safety
/// `handle` must be a valid pointer to a `uv_tty_t` initialized by `uv_tty_init`.
#[cfg(windows)]
pub unsafe extern "C" fn uv_tty_set_mode(
  handle: *mut uv_tty_t,
  mode: c_int,
) -> c_int {
  unsafe {
    let tty = &mut *handle;
    if mode == tty.internal_mode {
      return 0;
    }

    let h = tty.internal_handle as isize;
    let new_mode = match mode {
      UV_TTY_MODE_NORMAL => {
        windows_sys::Win32::System::Console::ENABLE_ECHO_INPUT
          | windows_sys::Win32::System::Console::ENABLE_LINE_INPUT
          | windows_sys::Win32::System::Console::ENABLE_PROCESSED_INPUT
      }
      UV_TTY_MODE_RAW | UV_TTY_MODE_IO => {
        windows_sys::Win32::System::Console::ENABLE_WINDOW_INPUT
      }
      _ => return UV_EINVAL,
    };

    if tty.flags & UV_HANDLE_TTY_READABLE != 0 {
      if windows_sys::Win32::System::Console::SetConsoleMode(h, new_mode) == 0 {
        return UV_EIO;
      }
    }

    tty.internal_mode = mode;
    0
  }
}

/// Get the current window size.
///
/// ### Safety
/// `handle` must be a valid pointer to a `uv_tty_t`. `width` and `height` must be valid,
/// writable pointers.
#[cfg(unix)]
pub unsafe extern "C" fn uv_tty_get_winsize(
  handle: *mut uv_tty_t,
  width: *mut c_int,
  height: *mut c_int,
) -> c_int {
  unsafe {
    let mut ws: libc::winsize = std::mem::zeroed();
    if libc::ioctl((*handle).internal_fd, libc::TIOCGWINSZ, &mut ws) < 0 {
      return UV_EIO;
    }
    *width = ws.ws_col as c_int;
    *height = ws.ws_row as c_int;
    0
  }
}

/// Get the current window size.
///
/// ### Safety
/// `handle` must be a valid pointer to a `uv_tty_t`. `width` and `height` must be valid,
/// writable pointers.
#[cfg(windows)]
pub unsafe extern "C" fn uv_tty_get_winsize(
  handle: *mut uv_tty_t,
  width: *mut c_int,
  height: *mut c_int,
) -> c_int {
  unsafe {
    let mut csbi: windows_sys::Win32::System::Console::CONSOLE_SCREEN_BUFFER_INFO =
      std::mem::zeroed();
    let h = (*handle).internal_handle as isize;
    if windows_sys::Win32::System::Console::GetConsoleScreenBufferInfo(
      h, &mut csbi,
    ) == 0
    {
      return UV_EIO;
    }
    *width = (csbi.srWindow.Right - csbi.srWindow.Left + 1) as c_int;
    *height = (csbi.srWindow.Bottom - csbi.srWindow.Top + 1) as c_int;
    0
  }
}

/// Reset the console to normal mode. Async-signal-safe.
///
/// ### Safety
/// This function accesses global state and should be called from signal handlers
/// or at process exit.
#[cfg(unix)]
pub unsafe extern "C" fn uv_tty_reset_mode() -> c_int {
  unsafe {
    // Spinlock acquire
    while TTY_RESET_LOCK
      .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
      .is_err()
    {
      std::hint::spin_loop();
    }

    let fd = TTY_RESET_FD;
    let status = if fd >= 0 {
      let termios_ptr =
        std::ptr::addr_of!(TTY_RESET_TERMIOS).cast::<libc::termios>();
      if libc::tcsetattr(fd, libc::TCSANOW, termios_ptr) == 0 {
        0
      } else {
        UV_EIO
      }
    } else {
      0
    };

    TTY_RESET_LOCK.store(false, Ordering::Release);
    status
  }
}

/// Reset the console to normal mode.
#[cfg(windows)]
pub unsafe extern "C" fn uv_tty_reset_mode() -> c_int {
  // On Windows this is typically handled by restoring the original console mode.
  // Since we don't have a global saved mode in this implementation, this is a no-op.
  0
}

/// Guess the handle type for a file descriptor.
///
/// ### Safety
/// `fd` must be a valid file descriptor or -1.
#[cfg(unix)]
pub unsafe extern "C" fn uv_guess_handle(fd: c_int) -> uv_handle_type {
  unsafe {
    if fd < 0 {
      return uv_handle_type::UV_UNKNOWN_HANDLE;
    }

    if libc::isatty(fd) != 0 {
      return uv_handle_type::UV_TTY;
    }

    let mut s: libc::stat = std::mem::zeroed();
    if libc::fstat(fd, &mut s) < 0 {
      return uv_handle_type::UV_UNKNOWN_HANDLE;
    }

    let mode = s.st_mode & libc::S_IFMT;
    if mode == libc::S_IFIFO {
      return uv_handle_type::UV_NAMED_PIPE;
    }
    if mode == libc::S_IFSOCK {
      return uv_handle_type::UV_TCP;
    }
    if mode == libc::S_IFREG {
      return uv_handle_type::UV_FILE;
    }

    uv_handle_type::UV_UNKNOWN_HANDLE
  }
}

/// Guess the handle type for a file descriptor.
///
/// ### Safety
/// `fd` must be a valid file descriptor or -1.
#[cfg(windows)]
pub unsafe extern "C" fn uv_guess_handle(fd: c_int) -> uv_handle_type {
  unsafe {
    if fd < 0 {
      return uv_handle_type::UV_UNKNOWN_HANDLE;
    }

    let os_handle = libc::get_osfhandle(fd);
    if os_handle == -1 {
      return uv_handle_type::UV_UNKNOWN_HANDLE;
    }

    let mut mode: u32 = 0;
    if windows_sys::Win32::System::Console::GetConsoleMode(
      os_handle as isize,
      &mut mode,
    ) != 0
    {
      return uv_handle_type::UV_TTY;
    }

    let file_type =
      windows_sys::Win32::Storage::FileSystem::GetFileType(os_handle as isize);
    if file_type == windows_sys::Win32::Storage::FileSystem::FILE_TYPE_PIPE {
      return uv_handle_type::UV_NAMED_PIPE;
    }
    if file_type == windows_sys::Win32::Storage::FileSystem::FILE_TYPE_DISK {
      return uv_handle_type::UV_FILE;
    }

    uv_handle_type::UV_UNKNOWN_HANDLE
  }
}

pub fn new_tty() -> UvTty {
  uv_tty_t {
    r#type: uv_handle_type::UV_TTY,
    loop_: std::ptr::null_mut(),
    data: std::ptr::null_mut(),
    flags: 0,
    internal_mode: UV_TTY_MODE_NORMAL,
    #[cfg(unix)]
    internal_fd: -1,
    #[cfg(unix)]
    internal_orig_termios: None,
    #[cfg(unix)]
    internal_async_fd: None,
    #[cfg(windows)]
    internal_handle: std::ptr::null_mut(),
    internal_alloc_cb: None,
    internal_read_cb: None,
    internal_reading: false,
    internal_write_queue: VecDeque::new(),
  }
}

pub fn new_tcp() -> UvTcp {
  uv_tcp_t {
    r#type: uv_handle_type::UV_TCP,
    loop_: std::ptr::null_mut(),
    data: std::ptr::null_mut(),
    flags: 0,
    internal_fd: None,
    internal_bind_addr: None,
    internal_stream: None,
    internal_listener: None,
    internal_listener_addr: None,
    internal_nodelay: false,
    internal_alloc_cb: None,
    internal_read_cb: None,
    internal_reading: false,
    internal_connect: None,
    internal_write_queue: VecDeque::new(),
    internal_connection_cb: None,
    internal_backlog: VecDeque::new(),
  }
}

pub fn new_write() -> UvWrite {
  uv_write_t {
    r#type: 0,
    data: std::ptr::null_mut(),
    handle: std::ptr::null_mut(),
  }
}

pub fn new_connect() -> UvConnect {
  uv_connect_t {
    r#type: 0,
    data: std::ptr::null_mut(),
    handle: std::ptr::null_mut(),
  }
}

pub fn new_shutdown() -> UvShutdown {
  uv_shutdown_t {
    r#type: 0,
    data: std::ptr::null_mut(),
    handle: std::ptr::null_mut(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Helper: create a loop, run a closure, close the loop.
  unsafe fn with_loop(f: impl FnOnce(*mut uv_loop_t)) {
    let mut uv_loop = std::mem::MaybeUninit::<uv_loop_t>::uninit();
    let lp = uv_loop.as_mut_ptr();
    // SAFETY: lp points to valid, writable memory.
    unsafe {
      uv_loop_init(lp);
      f(lp);
      uv_loop_close(lp);
    }
  }

  /// Open a PTY pair and return (master_fd, slave_fd).
  /// Returns None if openpty is not available (e.g. CI without a terminal).
  #[cfg(unix)]
  fn open_pty() -> Option<(c_int, c_int)> {
    let mut master: c_int = -1;
    let mut slave: c_int = -1;
    // SAFETY: pointers are valid, nulls are allowed for name/termios/winsize.
    let rc = unsafe {
      libc::openpty(
        &mut master,
        &mut slave,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
      )
    };
    if rc == 0 { Some((master, slave)) } else { None }
  }

  #[test]
  fn test_new_tty_defaults() {
    let tty = new_tty();
    assert_eq!(tty.r#type, uv_handle_type::UV_TTY);
    assert!(tty.loop_.is_null());
    assert_eq!(tty.internal_mode, UV_TTY_MODE_NORMAL);
    assert!(!tty.internal_reading);
    assert!(tty.internal_write_queue.is_empty());
  }

  #[test]
  fn test_tty_init_non_tty_fd_fails() {
    // A pipe fd is not a TTY, so uv_tty_init should fail with UV_EINVAL.
    #[cfg(unix)]
    unsafe {
      let mut fds = [0 as c_int; 2];
      assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);

      // Need a tokio runtime for AsyncFd
      let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
      let _guard = rt.enter();

      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, fds[0], 1);
        assert_eq!(rc, UV_EINVAL, "pipe fd should not be a TTY");
      });

      libc::close(fds[0]);
      libc::close(fds[1]);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_init_with_pty() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0, "uv_tty_init should succeed on PTY slave");
        assert_eq!(tty.r#type, uv_handle_type::UV_TTY);
        assert_eq!(tty.internal_fd, slave);
        assert!(tty.internal_orig_termios.is_some());

        // Clean up
        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_get_winsize_with_pty() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    // Set a known window size on the master
    unsafe {
      let ws = libc::winsize {
        ws_row: 25,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
      };
      libc::ioctl(master, libc::TIOCSWINSZ, &ws);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 0);
        assert_eq!(rc, 0);

        let mut width: c_int = 0;
        let mut height: c_int = 0;
        let rc = uv_tty_get_winsize(&mut tty, &mut width, &mut height);
        assert_eq!(rc, 0);
        assert_eq!(width, 80);
        assert_eq!(height, 25);

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_set_mode_raw_and_normal() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        // Switch to raw mode
        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_RAW);
        assert_eq!(rc, 0);
        assert_eq!(tty.internal_mode, UV_TTY_MODE_RAW);

        // Verify termios is actually in raw mode
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(slave, &mut t);
        assert_eq!(
          t.c_lflag & libc::ICANON,
          0,
          "ICANON should be off in raw mode"
        );
        assert_eq!(t.c_lflag & libc::ECHO, 0, "ECHO should be off in raw mode");

        // Switch back to normal
        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_NORMAL);
        assert_eq!(rc, 0);
        assert_eq!(tty.internal_mode, UV_TTY_MODE_NORMAL);

        // Setting same mode again is a no-op
        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_NORMAL);
        assert_eq!(rc, 0);

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_set_mode_io() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_IO);
        assert_eq!(rc, 0);
        assert_eq!(tty.internal_mode, UV_TTY_MODE_IO);

        // Verify cfmakeraw-like settings
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(slave, &mut t);
        assert_eq!(
          t.c_oflag & libc::OPOST,
          0,
          "OPOST should be off in IO mode"
        );
        assert_eq!(
          t.c_lflag & libc::ICANON,
          0,
          "ICANON should be off in IO mode"
        );

        // Back to normal
        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_NORMAL);
        assert_eq!(rc, 0);

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_set_mode_invalid() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        let rc = uv_tty_set_mode(&mut tty, 99);
        assert_eq!(rc, UV_EINVAL);

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_write_and_read() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();

    rt.block_on(async {
      unsafe {
        with_loop(|lp| {
          let inner = get_inner(lp);
          inner.set_waker(&futures::task::noop_waker());

          // --- Write side: init a TTY on master, write data ---
          let mut write_tty = new_tty();
          let rc = uv_tty_init(lp, &mut write_tty, master, 0);
          assert_eq!(rc, 0);

          let test_data = b"hello tty\n";
          let rc = uv_try_write(
            &mut write_tty as *mut uv_tty_t as *mut uv_stream_t,
            test_data,
          );
          // Should write some or all bytes (blocking writes on master PTY)
          assert!(rc > 0, "uv_try_write should succeed, got {rc}");

          // --- Read side: read from slave using libc::read ---
          // The PTY transforms the data, so just verify we get something
          let mut read_buf = [0u8; 256];
          // Set slave non-blocking for the read
          let fl = libc::fcntl(slave, libc::F_GETFL);
          libc::fcntl(slave, libc::F_SETFL, fl | libc::O_NONBLOCK);
          let n = libc::read(
            slave,
            read_buf.as_mut_ptr() as *mut c_void,
            read_buf.len(),
          );
          assert!(n > 0, "should read data from PTY slave, got {n}");

          uv_close(&mut write_tty as *mut uv_tty_t as *mut uv_handle_t, None);
          inner.run_close();
        });

        libc::close(master);
        libc::close(slave);
      }
    });
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_queued_write_drains_in_run_io() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();

    rt.block_on(async {
      unsafe {
        with_loop(|lp| {
          let inner = get_inner(lp);
          inner.set_waker(&futures::task::noop_waker());

          let mut tty = new_tty();
          let rc = uv_tty_init(lp, &mut tty, master, 0);
          assert_eq!(rc, 0);

          // Force blocking writes so run_io drains synchronously
          // (async path needs reactor ticks which aren't available
          // in a synchronous test)
          tty.flags |= UV_HANDLE_BLOCKING_WRITES;

          // Queue a write via uv_write
          static mut WRITE_CB_CALLED: bool = false;
          unsafe extern "C" fn write_cb(_req: *mut uv_write_t, status: i32) {
            assert_eq!(status, 0);
            // SAFETY: test-only global, single-threaded.
            unsafe {
              WRITE_CB_CALLED = true;
            }
          }

          let mut req = new_write();
          let data = b"queued write\n";
          let buf = uv_buf_t {
            base: data.as_ptr() as *mut c_char,
            len: data.len(),
          };
          let rc = uv_write(
            &mut req,
            &mut tty as *mut uv_tty_t as *mut uv_stream_t,
            &buf,
            1,
            Some(write_cb),
          );
          assert_eq!(rc, 0);
          assert!(!tty.internal_write_queue.is_empty());

          // Drain the write queue
          inner.run_io();
          assert!(WRITE_CB_CALLED, "write callback should have fired");

          uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
          inner.run_close();
        });

        libc::close(master);
        libc::close(slave);
      }
    });
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_close_callback() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let inner = get_inner(lp);

        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        static mut CLOSE_CB_CALLED: bool = false;
        unsafe extern "C" fn close_cb(_handle: *mut uv_handle_t) {
          // SAFETY: test-only global, single-threaded.
          unsafe {
            CLOSE_CB_CALLED = true;
          }
        }

        uv_close(
          &mut tty as *mut uv_tty_t as *mut uv_handle_t,
          Some(close_cb),
        );

        // Close callback fires on next run_close
        inner.run_close();
        assert!(CLOSE_CB_CALLED, "close callback should have fired");

        // Handle should no longer be in tty_handles
        assert!(inner.tty_handles.borrow().is_empty());
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_read_start_stop() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let inner = get_inner(lp);

        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        unsafe extern "C" fn alloc_cb(
          _handle: *mut uv_handle_t,
          _suggested_size: usize,
          _buf: *mut uv_buf_t,
        ) {
        }
        unsafe extern "C" fn read_cb(
          _stream: *mut uv_stream_t,
          _nread: isize,
          _buf: *const uv_buf_t,
        ) {
        }

        let stream = &mut tty as *mut uv_tty_t as *mut uv_stream_t;
        let rc = uv_read_start(stream, Some(alloc_cb), Some(read_cb));
        assert_eq!(rc, 0);
        assert!(tty.internal_reading);
        assert!(tty.flags & UV_HANDLE_ACTIVE != 0);

        let rc = uv_read_stop(stream);
        assert_eq!(rc, 0);
        assert!(!tty.internal_reading);

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_guess_handle() {
    unsafe {
      // Pipe should be detected as NAMED_PIPE
      let mut fds = [0 as c_int; 2];
      assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
      assert_eq!(uv_guess_handle(fds[0]), uv_handle_type::UV_NAMED_PIPE);
      assert_eq!(uv_guess_handle(fds[1]), uv_handle_type::UV_NAMED_PIPE);
      libc::close(fds[0]);
      libc::close(fds[1]);

      // Invalid fd
      assert_eq!(uv_guess_handle(-1), uv_handle_type::UV_UNKNOWN_HANDLE);

      // PTY should be detected as TTY
      if let Some((master, slave)) = open_pty() {
        assert_eq!(uv_guess_handle(master), uv_handle_type::UV_TTY);
        assert_eq!(uv_guess_handle(slave), uv_handle_type::UV_TTY);
        libc::close(master);
        libc::close(slave);
      }
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_reset_mode() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        // Save original termios for comparison
        let mut orig: libc::termios = std::mem::zeroed();
        libc::tcgetattr(slave, &mut orig);
        let orig_lflag = orig.c_lflag;

        // Switch to raw
        let rc = uv_tty_set_mode(&mut tty, UV_TTY_MODE_RAW);
        assert_eq!(rc, 0);

        // Verify it changed
        let mut current: libc::termios = std::mem::zeroed();
        libc::tcgetattr(slave, &mut current);
        assert_ne!(current.c_lflag & libc::ICANON, orig_lflag & libc::ICANON);

        // Manually set global state under the spinlock to ensure
        // it points to our fd (other parallel tests may overwrite it)
        while TTY_RESET_LOCK
          .compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
          )
          .is_err()
        {
          std::hint::spin_loop();
        }
        TTY_RESET_FD = tty.internal_fd;
        std::ptr::addr_of_mut!(TTY_RESET_TERMIOS)
          .cast::<libc::termios>()
          .write(orig);
        TTY_RESET_LOCK.store(false, Ordering::Release);

        // Global reset
        let rc = uv_tty_reset_mode();
        assert_eq!(rc, 0);

        // Verify restored
        let mut restored: libc::termios = std::mem::zeroed();
        libc::tcgetattr(tty.internal_fd, &mut restored);
        assert_eq!(
          restored.c_lflag & libc::ICANON,
          orig_lflag & libc::ICANON,
          "ICANON should be restored after reset_mode"
        );

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        let inner = get_inner(lp);
        inner.run_close();
      });

      libc::close(master);
      libc::close(slave);
    }
  }

  #[cfg(unix)]
  #[test]
  fn test_tty_has_alive_handles() {
    let Some((master, slave)) = open_pty() else {
      return;
    };

    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap();
    let _guard = rt.enter();

    unsafe {
      with_loop(|lp| {
        let inner = get_inner(lp);
        assert!(!inner.has_alive_handles());

        let mut tty = new_tty();
        let rc = uv_tty_init(lp, &mut tty, slave, 1);
        assert_eq!(rc, 0);

        // TTY is ref'd but not active yet
        assert!(!inner.has_alive_handles());

        // Make it active
        tty.flags |= UV_HANDLE_ACTIVE;
        assert!(inner.has_alive_handles());

        // Unref it
        uv_unref(&mut tty as *mut uv_tty_t as *mut uv_handle_t);
        assert!(!inner.has_alive_handles());

        // Re-ref
        uv_ref(&mut tty as *mut uv_tty_t as *mut uv_handle_t);
        assert!(inner.has_alive_handles());

        uv_close(&mut tty as *mut uv_tty_t as *mut uv_handle_t, None);
        inner.run_close();
        assert!(!inner.has_alive_handles());
      });

      libc::close(master);
      libc::close(slave);
    }
  }
}
