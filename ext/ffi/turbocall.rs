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
          // V8 object layout (no pointer compression, 64-bit).
          // See v8-internal.h Internals class and torque-generated
          // js-array-buffer-tq.inc for authoritative values.
          const {
            assert!(std::mem::size_of::<usize>() == 8);
            assert!(deno_core::v8::TYPED_ARRAY_MAX_SIZE_IN_HEAP == 0);
          }
          const HEAP_TAG: i32 = 1;
          const MAP_INSTANCE_TYPE_OFF: i32 = 12;
          const AB_BACKING_STORE_OFF: i32 = 56;
          const TA_EXTERNAL_PTR_OFF: i32 = 72;
          const FIRST_ABV_TYPE: i64 = 2059;
          const JS_AB_TYPE: i64 = 2062;

          // Deref Local handle -> tagged obj -> map -> instance_type
          let obj = f.ins().load(ISIZE, MemFlags::trusted(), arg, 0);
          let map = f.ins().load(ISIZE, MemFlags::trusted(), obj, -HEAP_TAG);
          let ty = f.ins().load(
            types::I16, MemFlags::trusted(), map,
            MAP_INSTANCE_TYPE_OFF - HEAP_TAG,
          );

          // instance_type in [2059..2062] covers ABV + AB
          let rel = f.ins().iadd_imm(ty, -FIRST_ABV_TYPE);
          let in_range = f.ins().icmp_imm(
            IntCC::UnsignedLessThanOrEqual, rel,
            JS_AB_TYPE - FIRST_ABV_TYPE, // 3
          );

          let inline_block = f.create_block();
          let merge = f.create_block();
          f.append_block_param(merge, ISIZE);
          let fallback = f.create_block();
          f.set_cold_block(fallback);

          f.ins().brif(in_range, inline_block, &[], fallback, &[]);

          // Branchless offset select: AB (rel==3) -> 56, ABV -> 72
          f.switch_to_block(inline_block);
          f.seal_block(inline_block);
          let is_ab = f.ins().icmp_imm(
            IntCC::Equal, rel, JS_AB_TYPE - FIRST_ABV_TYPE,
          );
          let ab_off = f.ins().iconst(ISIZE, (AB_BACKING_STORE_OFF - HEAP_TAG) as i64);
          let abv_off = f.ins().iconst(ISIZE, (TA_EXTERNAL_PTR_OFF - HEAP_TAG) as i64);
          let off = f.ins().select(is_ab, ab_off, abv_off);
          let addr = f.ins().iadd(obj, off);
          let ptr = f.ins().load(ISIZE, MemFlags::trusted(), addr, 0);
          f.ins().jump(merge, &[ptr]);

          f.switch_to_block(fallback);
          f.seal_block(fallback);
          let callee = f.ins().iconst(ISIZE, turbocall_ab_contents as usize as i64);
          let call = f.ins().call_indirect(ab_sig, callee, &[arg]);
          let result = f.inst_results(call)[0];
          let sentinel = f.ins().iconst(ISIZE, isize::MAX as i64);
          let is_err = f.ins().icmp(IntCC::Equal, result, sentinel);
          f.ins().brif(is_err, error, &[], merge, &[result]);

          f.switch_to_block(merge);
          f.seal_block(merge);
          f.def_var(target_v, f.block_params(merge)[0]);

          f.ins().jump(next, &[]);
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
