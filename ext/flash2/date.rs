use deno_core::op;
use deno_core::CancelFuture;
use deno_core::CancelHandle;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

pub(crate) struct DateLoopCancelHandle(pub(crate) Rc<CancelHandle>);

#[repr(transparent)]
pub struct HttpDate {
  pub current_date: String,
}

impl HttpDate {
  pub fn now() -> Self {
    Self {
      current_date: httpdate::fmt_http_date(SystemTime::now()),
    }
  }

  pub fn update(&mut self) {
    self.current_date = httpdate::fmt_http_date(SystemTime::now());
  }
}

#[op]
pub async fn op_flash_start_date_loop(state: Rc<RefCell<OpState>>) {
  let cancel_handle = {
    let s = state.borrow();
    let cancel_handle = s.borrow::<DateLoopCancelHandle>();
    cancel_handle.0.clone()
  };

  loop {
    let r = tokio::time::sleep(tokio::time::Duration::from_millis(1000))
      .or_cancel(&cancel_handle)
      .await;
    {
      let mut state = state.borrow_mut();
      let date = state.borrow_mut::<HttpDate>();
      date.update();
    }

    if r.is_err() {
      break;
    }
  }
}

#[op]
pub fn op_flash_stop_date_loop(state: &mut OpState) {
  let cancel_handle = state.borrow::<DateLoopCancelHandle>();
  cancel_handle.0.cancel();
}
