// Copyright 2018-2026 the Deno authors. MIT license.

use std::ffi::c_void;
use std::sync::LazyLock;

use deno_core::OpState;
use deno_core::op2;
use deno_core::v8;
use deno_core::v8::fast_api;

use crate::NativeType;
use crate::Symbol;
use crate::dlfcn::FunctionData;

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum TurbocallError {
  #[class(generic)]
  #[error(transparent)]
  SetError(#[from] cranelift::prelude::settings::SetError),

  #[class(generic)]
  #[error("Cranelift ISA error: {0}")]
  IsaError(&'static str),

  #[class(generic)]
  #[error(transparent)]
  CodegenError(#[from] cranelift::codegen::CodegenError),

  #[class(generic)]
  #[error(transparent)]
  VerifierError(#[from] cranelift::codegen::verifier::VerifierErrors),

  #[class(generic)]
  #[error("{0}")]
  CompileError(String),

  #[class(generic)]
  #[error(transparent)]
  Stdio(#[from] std::io::Error),
}

pub(crate) fn is_compatible(sym: &Symbol) -> bool {
  !matches!(sym.result_type, NativeType::Struct(_))
    && !sym
      .parameter_types
      .iter()
      .any(|t| matches!(t, NativeType::Struct(_)))
}

/// Check if a symbol is compatible with the slow-path JIT trampoline.
///
/// The slow trampoline inlines V8 type extraction and calls the FFI
/// function directly (no libffi). We skip types that require a
/// HandleScope (u64/i64/usize/isize/pointer/function) and structs.
pub(crate) fn is_slow_compatible(sym: &Symbol) -> bool {
  let params_ok = sym.parameter_types.iter().all(|t| {
    matches!(
      t,
      NativeType::Bool
        | NativeType::U8
        | NativeType::I8
        | NativeType::U16
        | NativeType::I16
        | NativeType::U32
        | NativeType::I32
        | NativeType::F32
        | NativeType::F64
        | NativeType::Buffer
    )
  });
  let result_ok = matches!(
    sym.result_type,
    NativeType::Void
      | NativeType::Bool
      | NativeType::U8
      | NativeType::I8
      | NativeType::U16
      | NativeType::I16
      | NativeType::U32
      | NativeType::I32
      | NativeType::F32
      | NativeType::F64
  );
  params_ok && result_ok
}

/// Trampoline for fast-call FFI functions
///
/// Calls the FFI function without the first argument (the receiver)
pub(crate) struct Trampoline(memmap2::Mmap);

impl Trampoline {
  pub(crate) fn ptr(&self) -> *const c_void {
    self.0.as_ptr() as *const c_void
  }
}

#[allow(unused)]
pub(crate) fn compile_trampoline(
  sym: &Symbol,
) -> Result<Trampoline, TurbocallError> {
  use cranelift::prelude::*;

  let mut flag_builder = settings::builder();
  flag_builder.set("is_pic", "true")?;
  flag_builder.set("opt_level", "speed_and_size")?;
  let flags = settings::Flags::new(flag_builder);

  let isa = cranelift_native::builder()
    .map_err(TurbocallError::IsaError)?
    .finish(flags)?;

  let mut wrapper_sig =
    cranelift::codegen::ir::Signature::new(isa.default_call_conv());
  let mut target_sig =
    cranelift::codegen::ir::Signature::new(isa.default_call_conv());
  let mut raise_sig =
    cranelift::codegen::ir::Signature::new(isa.default_call_conv());

  #[cfg(target_pointer_width = "32")]
  const ISIZE: Type = types::I32;
  #[cfg(target_pointer_width = "64")]
  const ISIZE: Type = types::I64;

  fn convert(t: &NativeType, wrapper: bool) -> AbiParam {
    match t {
      NativeType::U8 => {
        if wrapper {
          AbiParam::new(types::I32)
        } else {
          AbiParam::new(types::I8).uext()
        }
      }
      NativeType::I8 => {
        if wrapper {
          AbiParam::new(types::I32)
        } else {
          AbiParam::new(types::I8).sext()
        }
      }
      NativeType::U16 => {
        if wrapper {
          AbiParam::new(types::I32)
        } else {
          AbiParam::new(types::I16).uext()
        }
      }
      NativeType::I16 => {
        if wrapper {
          AbiParam::new(types::I32)
        } else {
          AbiParam::new(types::I16).sext()
        }
      }
      NativeType::Bool => {
        if wrapper {
          AbiParam::new(types::I32)
        } else {
          AbiParam::new(types::I8).uext()
        }
      }
      NativeType::U32 => AbiParam::new(types::I32),
      NativeType::I32 => AbiParam::new(types::I32),
      NativeType::U64 => AbiParam::new(types::I64),
      NativeType::I64 => AbiParam::new(types::I64),
      NativeType::USize => AbiParam::new(ISIZE),
      NativeType::ISize => AbiParam::new(ISIZE),
      NativeType::F32 => AbiParam::new(types::F32),
      NativeType::F64 => AbiParam::new(types::F64),
      NativeType::Pointer => AbiParam::new(ISIZE),
      NativeType::Buffer => AbiParam::new(ISIZE),
      NativeType::Function => AbiParam::new(ISIZE),
      NativeType::Struct(_) => AbiParam::new(types::INVALID),
      NativeType::Void => AbiParam::new(types::INVALID),
    }
  }

  // *const FastApiCallbackOptions
  raise_sig.params.push(AbiParam::new(ISIZE));

  // Local<Value> receiver
  wrapper_sig.params.push(AbiParam::new(ISIZE));

  for pty in &sym.parameter_types {
    target_sig.params.push(convert(pty, false));
    wrapper_sig.params.push(convert(pty, true));
  }

  // const FastApiCallbackOptions& options
  wrapper_sig.params.push(AbiParam::new(ISIZE));

  if !matches!(sym.result_type, NativeType::Struct(_) | NativeType::Void) {
    target_sig.returns.push(convert(&sym.result_type, false));
    wrapper_sig.returns.push(convert(&sym.result_type, true));
  }

  let mut ab_sig =
    cranelift::codegen::ir::Signature::new(isa.default_call_conv());
  ab_sig.params.push(AbiParam::new(ISIZE));
  ab_sig.returns.push(AbiParam::new(ISIZE));

  let mut ctx = cranelift::codegen::Context::new();
  let mut fn_builder_ctx = FunctionBuilderContext::new();

  ctx.func = cranelift::codegen::ir::Function::with_name_signature(
    cranelift::codegen::ir::UserFuncName::testcase(format!(
      "{}_wrapper",
      sym.name
    )),
    wrapper_sig,
  );

  let mut f = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

  let target_sig = f.import_signature(target_sig);
  let ab_sig = f.import_signature(ab_sig);
  let raise_sig = f.import_signature(raise_sig);

  {
    // Define blocks

    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);

    let error = f.create_block();
    f.set_cold_block(error);

    // Define variables

    let mut vidx = 0;
    for pt in &sym.parameter_types {
      let target_v = Variable::new(vidx);
      vidx += 1;

      let wrapper_v = Variable::new(vidx);
      vidx += 1;

      f.declare_var(target_v, convert(pt, false).value_type);
      f.declare_var(wrapper_v, convert(pt, true).value_type);
    }

    let options_v = Variable::new(vidx);
    vidx += 1;
    f.declare_var(options_v, ISIZE);

    // Go!

    f.switch_to_block(entry);
    f.seal_block(entry);

    let args = f.block_params(entry).to_owned();

    let mut vidx = 1;
    let mut argx = 1;
    for _ in &sym.parameter_types {
      f.def_var(Variable::new(vidx), args[argx]);
      argx += 1;
      vidx += 2;
    }

    f.def_var(options_v, args[argx]);

    static TRACE_TURBO: LazyLock<bool> = LazyLock::new(|| {
      std::env::var("DENO_UNSTABLE_FFI_TRACE_TURBO").as_deref() == Ok("1")
    });

    if *TRACE_TURBO {
      let options = f.use_var(options_v);
      let trace_fn = f.ins().iconst(ISIZE, turbocall_trace as usize as i64);
      f.ins().call_indirect(ab_sig, trace_fn, &[options]);
    }

    let mut next = f.create_block();

    let mut vidx = 0;
    for nty in &sym.parameter_types {
      let target_v = Variable::new(vidx);
      vidx += 1;
      let wrapper_v = Variable::new(vidx);
      vidx += 1;

      let arg = f.use_var(wrapper_v);

      match nty {
        NativeType::U8 | NativeType::I8 | NativeType::Bool => {
          let v = f.ins().ireduce(types::I8, arg);
          f.def_var(target_v, v);
        }
        NativeType::U16 | NativeType::I16 => {
          let v = f.ins().ireduce(types::I16, arg);
          f.def_var(target_v, v);
        }
        NativeType::Buffer => {
          let callee =
            f.ins().iconst(ISIZE, turbocall_ab_contents as usize as i64);
          let call = f.ins().call_indirect(ab_sig, callee, &[arg]);
          let result = f.inst_results(call)[0];
          f.def_var(target_v, result);

          let sentinel = f.ins().iconst(ISIZE, isize::MAX as i64);
          let condition = f.ins().icmp(IntCC::Equal, result, sentinel);
          f.ins().brif(condition, error, &[], next, &[]);

          // switch to new block
          f.switch_to_block(next);
          f.seal_block(next);
          next = f.create_block();
        }
        _ => {
          f.def_var(target_v, arg);
        }
      }
    }

    let mut args = Vec::with_capacity(sym.parameter_types.len());
    let mut vidx = 0;
    for _ in &sym.parameter_types {
      args.push(f.use_var(Variable::new(vidx)));
      vidx += 2; // skip wrapper arg
    }
    let callee = f.ins().iconst(ISIZE, sym.ptr.as_ptr() as i64);
    let call = f.ins().call_indirect(target_sig, callee, &args);
    let mut results = f.inst_results(call).to_owned();

    match sym.result_type {
      NativeType::U8 | NativeType::U16 | NativeType::Bool => {
        results[0] = f.ins().uextend(types::I32, results[0]);
      }
      NativeType::I8 | NativeType::I16 => {
        results[0] = f.ins().sextend(types::I32, results[0]);
      }
      _ => {}
    }

    f.ins().return_(&results);

    f.switch_to_block(error);
    f.seal_block(error);
    if !f.is_unreachable() {
      let options = f.use_var(options_v);
      let callee = f.ins().iconst(ISIZE, turbocall_raise as usize as i64);
      f.ins().call_indirect(raise_sig, callee, &[options]);
      let rty = convert(&sym.result_type, true);
      if rty.value_type.is_invalid() {
        f.ins().return_(&[]);
      } else {
        let zero = if rty.value_type == types::F32 {
          f.ins().f32const(0.0)
        } else if rty.value_type == types::F64 {
          f.ins().f64const(0.0)
        } else {
          f.ins().iconst(rty.value_type, 0)
        };
        f.ins().return_(&[zero]);
      }
    }
  }

  f.finalize();

  cranelift::codegen::verifier::verify_function(&ctx.func, isa.flags())?;

  let mut ctrl_plane = Default::default();
  ctx.optimize(&*isa, &mut ctrl_plane)?;

  log::trace!("Turbocall IR:\n{}", ctx.func.display());

  let code_info = ctx
    .compile(&*isa, &mut ctrl_plane)
    .map_err(|e| TurbocallError::CompileError(format!("{e:?}")))?;

  let data = code_info.buffer.data();
  let mut mutable = memmap2::MmapMut::map_anon(data.len())?;
  mutable.copy_from_slice(data);
  let buffer = mutable.make_exec()?;

  Ok(Trampoline(buffer))
}

pub(crate) struct Turbocall {
  pub trampoline: Trampoline,
  // Held in a box to keep the memory alive for CFunctionInfo
  #[allow(unused)]
  pub param_info: Box<[fast_api::CTypeInfo]>,
  // Held in a box to keep the memory alive for V8
  #[allow(unused)]
  pub c_function_info: Box<fast_api::CFunctionInfo>,
}

pub(crate) fn make_template(sym: &Symbol, trampoline: Trampoline) -> Turbocall {
  let param_info = std::iter::once(fast_api::Type::V8Value.as_info()) // Receiver
    .chain(sym.parameter_types.iter().map(|t| t.into()))
    .chain(std::iter::once(fast_api::Type::CallbackOptions.as_info()))
    .collect::<Box<_>>();

  let ret = if sym.result_type == NativeType::Buffer {
    // Buffer can be used as a return type and converts differently than in parameters.
    fast_api::Type::Pointer.as_info()
  } else {
    (&sym.result_type).into()
  };

  let c_function_info = Box::new(fast_api::CFunctionInfo::new(
    ret,
    &param_info,
    fast_api::Int64Representation::BigInt,
  ));

  Turbocall {
    trampoline,
    param_info,
    c_function_info,
  }
}

impl From<&NativeType> for fast_api::CTypeInfo {
  fn from(native_type: &NativeType) -> Self {
    match native_type {
      NativeType::Bool => fast_api::Type::Bool.as_info(),
      NativeType::U8 | NativeType::U16 | NativeType::U32 => {
        fast_api::Type::Uint32.as_info()
      }
      NativeType::I8 | NativeType::I16 | NativeType::I32 => {
        fast_api::Type::Int32.as_info()
      }
      NativeType::F32 => fast_api::Type::Float32.as_info(),
      NativeType::F64 => fast_api::Type::Float64.as_info(),
      NativeType::Void => fast_api::Type::Void.as_info(),
      NativeType::I64 => fast_api::Type::Int64.as_info(),
      NativeType::U64 => fast_api::Type::Uint64.as_info(),
      NativeType::ISize => fast_api::Type::Int64.as_info(),
      NativeType::USize => fast_api::Type::Uint64.as_info(),
      NativeType::Pointer | NativeType::Function => {
        fast_api::Type::Pointer.as_info()
      }
      NativeType::Buffer => fast_api::Type::V8Value.as_info(),
      NativeType::Struct(_) => fast_api::Type::V8Value.as_info(),
    }
  }
}

// --- Slow-path JIT trampoline helpers ---
//
// These extern "C" functions are called from the slow-path JIT trampoline
// to extract V8 argument values and set return values without a HandleScope.
// Each extraction helper takes a pointer to FunctionCallbackArguments, an
// argument index, and a pointer to an error flag. On type mismatch they
// set the error flag and return a zero value.

extern "C" fn slow_get_bool(
  args: *const c_void,
  index: i32,
  error: *mut bool,
) -> i32 {
  // SAFETY: args points to a valid FunctionCallbackArguments on the caller's stack.
  let args =
    unsafe { &*(args as *const v8::FunctionCallbackArguments<'static>) };
  let value = args.get(index);
  match v8::Local::<v8::Boolean>::try_from(value) {
    Ok(v) => v.is_true() as i32,
    Err(_) => {
      unsafe { *error = true };
      0
    }
  }
}

extern "C" fn slow_get_i32(
  args: *const c_void,
  index: i32,
  error: *mut bool,
) -> i32 {
  // SAFETY: args points to a valid FunctionCallbackArguments on the caller's stack.
  let args =
    unsafe { &*(args as *const v8::FunctionCallbackArguments<'static>) };
  let value = args.get(index);
  match v8::Local::<v8::Number>::try_from(value) {
    Ok(v) => v.value() as i32,
    Err(_) => {
      unsafe { *error = true };
      0
    }
  }
}

extern "C" fn slow_get_f64(
  args: *const c_void,
  index: i32,
  error: *mut bool,
) -> f64 {
  // SAFETY: args points to a valid FunctionCallbackArguments on the caller's stack.
  let args =
    unsafe { &*(args as *const v8::FunctionCallbackArguments<'static>) };
  let value = args.get(index);
  match v8::Local::<v8::Number>::try_from(value) {
    Ok(v) => v.value(),
    Err(_) => {
      unsafe { *error = true };
      0.0
    }
  }
}

extern "C" fn slow_get_buffer(
  args: *const c_void,
  index: i32,
  error: *mut bool,
) -> *mut c_void {
  // SAFETY: args points to a valid FunctionCallbackArguments on the caller's stack.
  let args =
    unsafe { &*(args as *const v8::FunctionCallbackArguments<'static>) };
  let value = args.get(index);
  match super::ir::parse_buffer_arg(value) {
    Ok(ptr) => ptr,
    Err(_) => {
      unsafe { *error = true };
      std::ptr::null_mut()
    }
  }
}

extern "C" fn slow_ret_i32(rv: *mut c_void, value: i32) {
  // SAFETY: rv points to a valid ReturnValue on the caller's stack.
  let rv = unsafe { &mut *(rv as *mut v8::ReturnValue) };
  rv.set_int32(value);
}

extern "C" fn slow_ret_u32(rv: *mut c_void, value: u32) {
  // SAFETY: rv points to a valid ReturnValue on the caller's stack.
  let rv = unsafe { &mut *(rv as *mut v8::ReturnValue) };
  rv.set_uint32(value);
}

extern "C" fn slow_ret_f64(rv: *mut c_void, value: f64) {
  // SAFETY: rv points to a valid ReturnValue on the caller's stack.
  let rv = unsafe { &mut *(rv as *mut v8::ReturnValue) };
  rv.set_double(value);
}

/// The type of a compiled slow-path JIT trampoline function.
///
/// Arguments: (args_ptr, rv_ptr) -> success (nonzero = ok)
///
/// `args_ptr` is a pointer to the `FunctionCallbackArguments` on the
/// caller's stack. `rv_ptr` is a pointer to the `ReturnValue`.
/// Returns nonzero on success. On failure (type extraction error),
/// returns 0 and the caller should fall back to the generic slow path
/// which will produce the proper JS exception.
pub(crate) type SlowTrampolineFn =
  extern "C" fn(args: *const c_void, rv: *mut c_void) -> u8;

/// Compile a slow-path JIT trampoline for the given symbol.
///
/// The trampoline extracts V8 argument values via helper calls,
/// calls the FFI function directly (bypassing libffi), and sets
/// the return value on the ReturnValue — all without a HandleScope.
pub(crate) fn compile_slow_trampoline(
  sym: &Symbol,
) -> Result<Trampoline, TurbocallError> {
  use cranelift::prelude::*;

  let mut flag_builder = settings::builder();
  flag_builder.set("is_pic", "true")?;
  flag_builder.set("opt_level", "speed_and_size")?;
  let flags = settings::Flags::new(flag_builder);

  let isa = cranelift_native::builder()
    .map_err(TurbocallError::IsaError)?
    .finish(flags)?;

  #[cfg(target_pointer_width = "32")]
  const ISIZE: Type = types::I32;
  #[cfg(target_pointer_width = "64")]
  const ISIZE: Type = types::I64;

  fn convert(t: &NativeType) -> AbiParam {
    match t {
      NativeType::U8 => AbiParam::new(types::I8).uext(),
      NativeType::I8 => AbiParam::new(types::I8).sext(),
      NativeType::U16 => AbiParam::new(types::I16).uext(),
      NativeType::I16 => AbiParam::new(types::I16).sext(),
      NativeType::Bool => AbiParam::new(types::I8).uext(),
      NativeType::U32 => AbiParam::new(types::I32),
      NativeType::I32 => AbiParam::new(types::I32),
      NativeType::U64 => AbiParam::new(types::I64),
      NativeType::I64 => AbiParam::new(types::I64),
      NativeType::USize => AbiParam::new(ISIZE),
      NativeType::ISize => AbiParam::new(ISIZE),
      NativeType::F32 => AbiParam::new(types::F32),
      NativeType::F64 => AbiParam::new(types::F64),
      NativeType::Pointer | NativeType::Buffer | NativeType::Function => {
        AbiParam::new(ISIZE)
      }
      NativeType::Struct(_) | NativeType::Void => {
        AbiParam::new(types::INVALID)
      }
    }
  }

  let cc = isa.default_call_conv();

  // Slow trampoline: (args_ptr: ISIZE, rv_ptr: ISIZE) -> I8
  let mut trampoline_sig =
    cranelift::codegen::ir::Signature::new(cc);
  trampoline_sig.params.push(AbiParam::new(ISIZE));
  trampoline_sig.params.push(AbiParam::new(ISIZE));
  trampoline_sig.returns.push(AbiParam::new(types::I8));

  // Target FFI function signature
  let mut target_sig = cranelift::codegen::ir::Signature::new(cc);
  for pty in &sym.parameter_types {
    target_sig.params.push(convert(pty));
  }
  if !matches!(sym.result_type, NativeType::Void) {
    target_sig.returns.push(convert(&sym.result_type));
  }

  let mut ctx = cranelift::codegen::Context::new();
  let mut fn_builder_ctx = FunctionBuilderContext::new();

  ctx.func = cranelift::codegen::ir::Function::with_name_signature(
    cranelift::codegen::ir::UserFuncName::testcase(format!(
      "{}_slow",
      sym.name
    )),
    trampoline_sig,
  );

  let mut f = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
  let target_sig = f.import_signature(target_sig);

  // Helper signatures — import all unconditionally.
  // get_i32 / get_bool: (args_ptr, index, error_ptr) -> i32
  let sig_get_i32 = {
    let mut sig = cranelift::codegen::ir::Signature::new(cc);
    sig.params.push(AbiParam::new(ISIZE));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ISIZE));
    sig.returns.push(AbiParam::new(types::I32));
    f.import_signature(sig)
  };
  // get_f64: (args_ptr, index, error_ptr) -> f64
  let sig_get_f64 = {
    let mut sig = cranelift::codegen::ir::Signature::new(cc);
    sig.params.push(AbiParam::new(ISIZE));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ISIZE));
    sig.returns.push(AbiParam::new(types::F64));
    f.import_signature(sig)
  };
  // get_buffer: (args_ptr, index, error_ptr) -> isize
  let sig_get_ptr = {
    let mut sig = cranelift::codegen::ir::Signature::new(cc);
    sig.params.push(AbiParam::new(ISIZE));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ISIZE));
    sig.returns.push(AbiParam::new(ISIZE));
    f.import_signature(sig)
  };
  // ret_i32 / ret_u32: (rv_ptr, value)
  let sig_ret_i32 = {
    let mut sig = cranelift::codegen::ir::Signature::new(cc);
    sig.params.push(AbiParam::new(ISIZE));
    sig.params.push(AbiParam::new(types::I32));
    f.import_signature(sig)
  };
  // ret_f64: (rv_ptr, value)
  let sig_ret_f64 = {
    let mut sig = cranelift::codegen::ir::Signature::new(cc);
    sig.params.push(AbiParam::new(ISIZE));
    sig.params.push(AbiParam::new(types::F64));
    f.import_signature(sig)
  };

  let has_params = !sym.parameter_types.is_empty();

  {
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);

    f.switch_to_block(entry);
    f.seal_block(entry);

    let params = f.block_params(entry).to_owned();
    let args_ptr = params[0];
    let rv_ptr = params[1];

    let mut target_args = Vec::with_capacity(sym.parameter_types.len());

    let error_block = if has_params {
      let eb = f.create_block();
      f.set_cold_block(eb);
      Some(eb)
    } else {
      None
    };
    let call_block = if has_params {
      Some(f.create_block())
    } else {
      None
    };

    // Error flag on stack
    let error_slot = if has_params {
      let slot = f.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        1,
        0,
      ));
      let zero_i8 = f.ins().iconst(types::I8, 0);
      f.ins().stack_store(zero_i8, slot, 0);
      let addr = f.ins().stack_addr(ISIZE, slot, 0);
      Some((slot, addr))
    } else {
      None
    };

    // Extract each parameter via external helper call.
    for (index, nty) in sym.parameter_types.iter().enumerate() {
      let idx = f.ins().iconst(types::I32, index as i64);
      let err = error_slot.unwrap().1;

      match nty {
        NativeType::Bool => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_bool as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_i32, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          let v = f.ins().ireduce(types::I8, v);
          target_args.push(v);
        }
        NativeType::U8 | NativeType::I8 => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_i32 as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_i32, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          let v = f.ins().ireduce(types::I8, v);
          target_args.push(v);
        }
        NativeType::U16 | NativeType::I16 => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_i32 as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_i32, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          let v = f.ins().ireduce(types::I16, v);
          target_args.push(v);
        }
        NativeType::U32 | NativeType::I32 => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_i32 as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_i32, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          target_args.push(v);
        }
        NativeType::F32 => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_f64 as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_f64, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          let v = f.ins().fdemote(types::F32, v);
          target_args.push(v);
        }
        NativeType::F64 => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_f64 as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_f64, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          target_args.push(v);
        }
        NativeType::Buffer => {
          let callee =
            f.ins().iconst(ISIZE, slow_get_buffer as usize as i64);
          let call = f.ins().call_indirect(
            sig_get_ptr, callee, &[args_ptr, idx, err],
          );
          let v = f.inst_results(call)[0];
          target_args.push(v);
        }
        _ => unreachable!("is_slow_compatible should have filtered this"),
      }
    }

    // Check error flag.
    if has_params {
      let (slot, _) = error_slot.unwrap();
      let e = f.ins().stack_load(types::I8, slot, 0);
      let is_err = f.ins().icmp_imm(IntCC::NotEqual, e, 0);
      f.ins().brif(
        is_err,
        error_block.unwrap(),
        &[],
        call_block.unwrap(),
        &[],
      );
      f.switch_to_block(call_block.unwrap());
      f.seal_block(call_block.unwrap());
    }

    // Call the FFI target.
    let callee = f.ins().iconst(ISIZE, sym.ptr.as_ptr() as i64);
    let call = f.ins().call_indirect(target_sig, callee, &target_args);

    // Set the return value via helper.
    match &sym.result_type {
      NativeType::Void => {}
      NativeType::Bool | NativeType::U8 | NativeType::U16 => {
        let result = f.inst_results(call)[0];
        let v = f.ins().uextend(types::I32, result);
        let callee =
          f.ins().iconst(ISIZE, slow_ret_u32 as usize as i64);
        f.ins().call_indirect(sig_ret_i32, callee, &[rv_ptr, v]);
      }
      NativeType::U32 => {
        let result = f.inst_results(call)[0];
        let callee =
          f.ins().iconst(ISIZE, slow_ret_u32 as usize as i64);
        f.ins().call_indirect(sig_ret_i32, callee, &[rv_ptr, result]);
      }
      NativeType::I8 | NativeType::I16 => {
        let result = f.inst_results(call)[0];
        let v = f.ins().sextend(types::I32, result);
        let callee =
          f.ins().iconst(ISIZE, slow_ret_i32 as usize as i64);
        f.ins().call_indirect(sig_ret_i32, callee, &[rv_ptr, v]);
      }
      NativeType::I32 => {
        let result = f.inst_results(call)[0];
        let callee =
          f.ins().iconst(ISIZE, slow_ret_i32 as usize as i64);
        f.ins().call_indirect(sig_ret_i32, callee, &[rv_ptr, result]);
      }
      NativeType::F32 => {
        let result = f.inst_results(call)[0];
        let v = f.ins().fpromote(types::F64, result);
        let callee =
          f.ins().iconst(ISIZE, slow_ret_f64 as usize as i64);
        f.ins().call_indirect(sig_ret_f64, callee, &[rv_ptr, v]);
      }
      NativeType::F64 => {
        let result = f.inst_results(call)[0];
        let callee =
          f.ins().iconst(ISIZE, slow_ret_f64 as usize as i64);
        f.ins()
          .call_indirect(sig_ret_f64, callee, &[rv_ptr, result]);
      }
      _ => unreachable!("is_slow_compatible should have filtered this"),
    }

    let one = f.ins().iconst(types::I8, 1);
    f.ins().return_(&[one]);

    if has_params {
      f.switch_to_block(error_block.unwrap());
      f.seal_block(error_block.unwrap());
      let zero = f.ins().iconst(types::I8, 0);
      f.ins().return_(&[zero]);
    }
  }

  f.finalize();

  cranelift::codegen::verifier::verify_function(&ctx.func, isa.flags())?;

  let mut ctrl_plane = Default::default();
  ctx.optimize(&*isa, &mut ctrl_plane)?;

  log::trace!("Slow turbocall IR:\n{}", ctx.func.display());

  let code_info = ctx
    .compile(&*isa, &mut ctrl_plane)
    .map_err(|e| TurbocallError::CompileError(format!("{e:?}")))?;

  let data = code_info.buffer.data();
  let mut mutable = memmap2::MmapMut::map_anon(data.len())?;
  mutable.copy_from_slice(data);
  let buffer = mutable.make_exec()?;

  Ok(Trampoline(buffer))
}

extern "C" fn turbocall_ab_contents(
  v: deno_core::v8::Local<deno_core::v8::Value>,
) -> *mut c_void {
  super::ir::parse_buffer_arg(v).unwrap_or(isize::MAX as _)
}

unsafe extern "C" fn turbocall_raise(
  options: *const deno_core::v8::fast_api::FastApiCallbackOptions,
) {
  // SAFETY: This is called with valid FastApiCallbackOptions from within fast callback.
  v8::callback_scope!(unsafe scope, unsafe { &*options });
  let exception =
    deno_core::error::to_v8_error(scope, &crate::IRError::InvalidBufferType);
  scope.throw_exception(exception);
}

pub struct TurbocallTarget(String);

unsafe extern "C" fn turbocall_trace(
  options: *const deno_core::v8::fast_api::FastApiCallbackOptions,
) {
  // SAFETY: This is called with valid FastApiCallbackOptions from within fast callback.
  v8::callback_scope!(unsafe let scope, unsafe { &*options });
  let func_data = deno_core::cppgc::try_unwrap_cppgc_object::<FunctionData>(
    scope,
    // SAFETY: This is valid if the options are valid.
    unsafe { (&*options).data },
  )
  .unwrap();
  deno_core::JsRuntime::op_state_from(scope)
    .borrow_mut()
    .put(TurbocallTarget(func_data.symbol.name.clone()));
}

#[op2]
#[string]
pub fn op_ffi_get_turbocall_target(state: &mut OpState) -> Option<String> {
  state.try_take::<TurbocallTarget>().map(|t| t.0)
}
