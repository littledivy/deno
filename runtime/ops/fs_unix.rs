use std::io::Error; 
use std::io::ErrorKind;
use std::io::Result;
use std::os::unix::io::RawFd;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::unix::AsyncFd;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

pub struct AsyncFile {
  fd: AsyncFd<RawFd>,
}

impl AsyncFile {
  pub fn from_file(fd: RawFd) -> Result<AsyncFile> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
      return Err(Error::last_os_error());
    }

    match unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } {
      0 => Ok(AsyncFile {
        fd: AsyncFd::new(fd)?,
      }),
      _ => Err(Error::last_os_error()),
    }
  }
}

impl AsyncRead for AsyncFile {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<Result<()>> {
    loop {
      let mut ready = match self.fd.poll_read_ready(cx) {
        Poll::Ready(x) => x?,
        Poll::Pending => return Poll::Pending,
      };

      let ret = unsafe {
        libc::read(
          *self.fd.get_ref(),
          buf.unfilled_mut() as *mut _ as _,
          buf.remaining(),
        )
      };

      return if ret < 0 {
        let e = Error::last_os_error();
        if e.kind() == ErrorKind::WouldBlock {
          ready.clear_ready();
          continue;
        } else {
          Poll::Ready(Err(e))
        }
      } else {
        let n = ret as usize;
        unsafe { buf.assume_init(n) };
        buf.advance(n);
        Poll::Ready(Ok(()))
      };
    }
  }
}

impl AsyncWrite for AsyncFile {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<Result<usize>> {
    loop {
      let mut ready = match self.fd.poll_write_ready(cx) {
        Poll::Ready(x) => x?,
        Poll::Pending => return Poll::Pending,
      };

      let ret = unsafe {
        libc::write(*self.fd.get_ref(), buf.as_ptr() as _, buf.len())
      };

      return if ret < 0 {
        let e = Error::last_os_error();
        if e.kind() == ErrorKind::WouldBlock {
          ready.clear_ready();
          continue;
        } else {
          Poll::Ready(Err(e))
        }
      } else {
        Poll::Ready(Ok(ret as usize))
      };
    }
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>,
  ) -> Poll<Result<()>> {
    Poll::Ready(Ok(()))
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>,
  ) -> Poll<Result<()>> {
    Poll::Ready(Ok(()))
  }
}
