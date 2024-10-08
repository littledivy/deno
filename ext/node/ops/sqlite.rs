// Copyright 2018-2024 the Deno authors. All rights reserved. MIT license.

use std::ops::Deref;
use std::rc::Rc;

use deno_core::error::AnyError;
use deno_core::op2;
use deno_core::v8;
use deno_core::GarbageCollected;

struct DatabaseSync {
  conn: Rc<rusqlite::Connection>,
}

impl DatabaseSync {
  pub fn new(conn: rusqlite::Connection) -> Self {
    Self {
      conn: Rc::new(conn),
    }
  }
}

impl Deref for DatabaseSync {
  type Target = rusqlite::Connection;

  fn deref(&self) -> &Self::Target {
    &self.conn
  }
}

impl GarbageCollected for DatabaseSync {}

#[op2]
#[cppgc]
pub fn op_sqlite_open(
  #[string] location: &str,
  enable_foreign_keys_on_open: bool,
) -> Result<DatabaseSync, AnyError> {
  let db = rusqlite::Connection::open(location)?;
  if enable_foreign_keys_on_open {
    db.execute("PRAGMA foreign_keys = ON", [])?;
  }

  Ok(DatabaseSync::new(db))
}

#[op2(fast)]
pub fn op_sqlite_exec_noargs(
  #[cppgc] db: &DatabaseSync,
  #[string] sql: &str,
) -> Result<(), AnyError> {
  db.execute(sql, [])?;
  Ok(())
}

#[op2(nofast)] // wtf??
pub fn op_sqlite_exec<'s>(
  scope: &mut v8::HandleScope<'s>,
  #[cppgc] db: &DatabaseSync,
  #[string] sql: &str,
  params: v8::Local<'s, v8::Array>,
) -> Result<(), AnyError> {
  let mut stmt = db.prepare_cached(sql)?;
  bind(&mut stmt, scope, params)?;

  Ok(())
}

struct StatementSync {
  inner: *mut libsqlite3_sys::sqlite3_stmt,
  use_big_ints: bool,
  allow_bare_named_params: bool,
  db: Rc<rusqlite::Connection>,
}

impl GarbageCollected for StatementSync {}

#[op2]
#[cppgc]
pub fn op_sqlite_prepare(
  #[cppgc] db: &DatabaseSync,
  #[string] sql: &str,
) -> Result<StatementSync, AnyError> {
  let raw_handle = unsafe { db.handle() };

  let mut raw_stmt = std::ptr::null_mut();
  let r = unsafe {
    libsqlite3_sys::sqlite3_prepare_v2(
      raw_handle,
      sql.as_ptr() as *const i8,
      sql.len() as i32,
      &mut raw_stmt,
      std::ptr::null_mut(),
    )
  };

  if r != libsqlite3_sys::SQLITE_OK {
    return Err(AnyError::msg("Failed to prepare statement"));
  }

  Ok(StatementSync {
    inner: raw_stmt,
    use_big_ints: false,
    allow_bare_named_params: false,
    db: Rc::clone(&db.conn),
  })
}

#[op2]
#[string]
pub fn op_sqlite_expandedsql(
  #[cppgc] stmt: &StatementSync,
) -> Result<String, AnyError> {
  let raw = unsafe { libsqlite3_sys::sqlite3_expanded_sql(stmt.inner) };
  if raw.is_null() {
    return Err(AnyError::msg("Failed to expand SQL"));
  }

  let cstr = unsafe { std::ffi::CStr::from_ptr(raw) };
  let expanded_sql = cstr.to_string_lossy().to_string();

  unsafe { libsqlite3_sys::sqlite3_free(raw as *mut std::ffi::c_void) };

  Ok(expanded_sql)
}

#[op2]
pub fn op_sqlite_get<'a>(
  scope: &mut v8::HandleScope<'a>,
  #[cppgc] stmt: &StatementSync,
  params: v8::Local<'a, v8::Array>,
) -> Result<v8::Local<'a, v8::Object>, AnyError> {
  let raw = stmt.inner;

  let result = v8::Object::new(scope);
  unsafe {
    libsqlite3_sys::sqlite3_reset(raw);

    let r = libsqlite3_sys::sqlite3_step(raw);

    if r == libsqlite3_sys::SQLITE_DONE {
      return Ok(v8::Object::new(scope));
    }
    if r != libsqlite3_sys::SQLITE_ROW {
      return Err(AnyError::msg("Failed to step statement"));
    }

    let columns = libsqlite3_sys::sqlite3_column_count(raw);

    for i in 0..columns {
      let name = libsqlite3_sys::sqlite3_column_name(raw, i);
      let name = std::ffi::CStr::from_ptr(name).to_string_lossy().to_string();
      let value = match libsqlite3_sys::sqlite3_column_type(raw, i) {
        libsqlite3_sys::SQLITE_INTEGER => {
          let value = libsqlite3_sys::sqlite3_column_int64(raw, i);
          v8::Integer::new(scope, value as _).into()
        }
        libsqlite3_sys::SQLITE_FLOAT => {
          let value = libsqlite3_sys::sqlite3_column_double(raw, i);
          v8::Number::new(scope, value).into()
        }
        libsqlite3_sys::SQLITE_TEXT => {
          let value = libsqlite3_sys::sqlite3_column_text(raw, i);
          let value = std::ffi::CStr::from_ptr(value as _)
            .to_string_lossy()
            .to_string();
          v8::String::new_from_utf8(
            scope,
            value.as_bytes(),
            v8::NewStringType::Normal,
          )
          .unwrap()
          .into()
        }
        libsqlite3_sys::SQLITE_BLOB => {
          let value = libsqlite3_sys::sqlite3_column_blob(raw, i);
          let size = libsqlite3_sys::sqlite3_column_bytes(raw, i);
          let value =
            std::slice::from_raw_parts(value as *const u8, size as usize);
          let value =
            v8::ArrayBuffer::new_backing_store_from_vec(value.to_vec())
              .make_shared();
          v8::ArrayBuffer::with_backing_store(scope, &value).into()
        }
        libsqlite3_sys::SQLITE_NULL => v8::null(scope).into(),
        _ => {
          return Err(AnyError::msg("Unknown column type"));
        }
      };

      let name = v8::String::new_from_utf8(
        scope,
        name.as_bytes(),
        v8::NewStringType::Normal,
      )
      .unwrap()
      .into();
      result.set(scope, name, value);
    }
  }

  Ok(result)
}

fn bind<'s>(
  stmt: &mut rusqlite::Statement,
  scope: &mut v8::HandleScope<'s>,
  params: v8::Local<'s, v8::Array>,
) -> Result<(), AnyError> {
  for index in 0..params.length() as usize {
    let value = params.get_index(scope, index as u32).unwrap();
    let index = index + 1;
    if value.is_null() {
      // stmt.raw_bind_parameter(index, ())?;
    } else if value.is_boolean() {
      stmt.raw_bind_parameter(index, value.is_true())?;
    } else if value.is_int32() {
      stmt.raw_bind_parameter(index, value.integer_value(scope).unwrap())?;
    } else if value.is_number() {
      stmt.raw_bind_parameter(index, value.number_value(scope).unwrap())?;
    } else if value.is_big_int() {
      let bigint = value.to_big_int(scope).unwrap();
      let (value, _) = bigint.i64_value();
      stmt.raw_bind_parameter(index, value)?;
    } else if value.is_string() {
      stmt.raw_bind_parameter(index, value.to_rust_string_lossy(scope))?;
    }
    // TODO: Blobs
  }

  Ok(())
}
