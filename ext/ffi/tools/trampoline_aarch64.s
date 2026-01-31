// Copyright 2018-2026 the Deno authors. MIT license.
//
// Hand-written aarch64 trampoline prototypes for FFI slow calls.
// These serve as reference implementations and optimization targets
// for the Cranelift JIT in turbocall.rs.
//
// Build & test (macOS):
//   as -o trampoline_aarch64.o trampoline_aarch64.s
//   // link with a test harness to validate behavior
//
// Calling convention (Apple aarch64):
//   args:    x0-x7 (integer/pointer), d0-d7 (float/double)
//   return:  x0 (integer/pointer), d0 (float/double)
//   caller-saved: x9-x15, d16-d31
//   callee-saved: x19-x28, d8-d15
//
// V8 FunctionCallbackInfo layout (slow callback):
//   args[0]: implicit_args pointer  (x0 in our trampoline)
//   args[1]: values pointer
//   args[2]: length
//   We receive: x0 = &FunctionCallbackArguments, x1 = &ReturnValue
//
// Slow trampoline signature:
//   extern "C" fn(args: *const c_void, rv: *mut c_void) -> u8
//   Returns 1 on success, 0 on type error (caller falls back to generic path)

.text
.align 4

// ---------------------------------------------------------------------------
// slow_trampoline_nop_void
//
// Simplest case: zero-arg void-return FFI call.
// No arg extraction, no return value to set, just call through.
//
// fn nop() -> void
// ---------------------------------------------------------------------------
.globl _slow_trampoline_nop_void
_slow_trampoline_nop_void:
    // x0 = args (unused), x1 = rv (unused)
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // Load FFI target address (patched at JIT time)
    adrp    x9, _ffi_target_nop@PAGE
    ldr     x9, [x9, _ffi_target_nop@PAGEOFF]
    blr     x9

    // Return 1 (success) — V8 defaults ReturnValue to undefined
    mov     w0, #1
    ldp     x29, x30, [sp], #16
    ret

// ---------------------------------------------------------------------------
// slow_trampoline_add_u32
//
// Two i32 args, i32 return. Demonstrates the arg extraction pattern:
// each arg is pulled from FunctionCallbackArguments via a helper call,
// with an error flag on the stack.
//
// fn add_u32(a: u32, b: u32) -> u32
// ---------------------------------------------------------------------------
.globl _slow_trampoline_add_u32
_slow_trampoline_add_u32:
    stp     x29, x30, [sp, #-48]!
    mov     x29, sp
    stp     x19, x20, [sp, #16]
    stp     x21, x22, [sp, #32]        // save callee-saved regs

    mov     x19, x0                     // x19 = args ptr
    mov     x20, x1                     // x20 = rv ptr

    // --- error flag on stack (1 byte, aligned) ---
    sub     sp, sp, #16
    strb    wzr, [sp]                   // error = false

    // --- extract arg 0: i32 ---
    mov     x0, x19                     // args
    mov     w1, #0                      // index = 0
    add     x2, sp, #0                  // &error
    adrp    x9, _slow_get_i32@PAGE
    ldr     x9, [x9, _slow_get_i32@PAGEOFF]
    blr     x9
    mov     w21, w0                     // stash arg0

    // --- extract arg 1: i32 ---
    mov     x0, x19                     // args
    mov     w1, #1                      // index = 1
    add     x2, sp, #0                  // &error
    adrp    x9, _slow_get_i32@PAGE
    ldr     x9, [x9, _slow_get_i32@PAGEOFF]
    blr     x9
    mov     w22, w0                     // stash arg1

    // --- check error flag ---
    ldrb    w9, [sp]
    cbnz    w9, .Ladd_u32_error

    // --- call FFI target: add_u32(arg0, arg1) ---
    mov     w0, w21
    mov     w1, w22
    adrp    x9, _ffi_target_add_u32@PAGE
    ldr     x9, [x9, _ffi_target_add_u32@PAGEOFF]
    blr     x9
    // w0 = result

    // --- set return value: ReturnValue::set_uint32(rv, result) ---
    mov     w1, w0                      // result
    mov     x0, x20                     // rv ptr
    adrp    x9, _slow_ret_u32@PAGE
    ldr     x9, [x9, _slow_ret_u32@PAGEOFF]
    blr     x9

    // success
    add     sp, sp, #16
    mov     w0, #1
    ldp     x21, x22, [sp, #32]
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #48
    ret

.Ladd_u32_error:
    // type extraction failed — return 0, caller falls back to generic path
    add     sp, sp, #16
    mov     w0, #0
    ldp     x21, x22, [sp, #32]
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #48
    ret

// ---------------------------------------------------------------------------
// slow_trampoline_hash
//
// Mixed types: buffer + u32 -> u32. Shows pointer arg extraction via
// slow_get_buffer which returns the backing store pointer.
//
// fn hash(buf: *const u8, len: u32) -> u32
// ---------------------------------------------------------------------------
.globl _slow_trampoline_hash
_slow_trampoline_hash:
    stp     x29, x30, [sp, #-48]!
    mov     x29, sp
    stp     x19, x20, [sp, #16]
    stp     x21, x22, [sp, #32]

    mov     x19, x0                     // args
    mov     x20, x1                     // rv

    sub     sp, sp, #16
    strb    wzr, [sp]                   // error = false

    // --- extract arg 0: buffer -> pointer ---
    mov     x0, x19
    mov     w1, #0
    add     x2, sp, #0
    adrp    x9, _slow_get_buffer@PAGE
    ldr     x9, [x9, _slow_get_buffer@PAGEOFF]
    blr     x9
    mov     x21, x0                     // buf ptr

    // --- extract arg 1: u32 ---
    mov     x0, x19
    mov     w1, #1
    add     x2, sp, #0
    adrp    x9, _slow_get_i32@PAGE
    ldr     x9, [x9, _slow_get_i32@PAGEOFF]
    blr     x9
    mov     w22, w0                     // len

    ldrb    w9, [sp]
    cbnz    w9, .Lhash_error

    // --- call FFI target: hash(buf, len) ---
    mov     x0, x21
    mov     w1, w22
    adrp    x9, _ffi_target_hash@PAGE
    ldr     x9, [x9, _ffi_target_hash@PAGEOFF]
    blr     x9

    // --- set return: u32 ---
    mov     w1, w0
    mov     x0, x20
    adrp    x9, _slow_ret_u32@PAGE
    ldr     x9, [x9, _slow_ret_u32@PAGEOFF]
    blr     x9

    add     sp, sp, #16
    mov     w0, #1
    ldp     x21, x22, [sp, #32]
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #48
    ret

.Lhash_error:
    add     sp, sp, #16
    mov     w0, #0
    ldp     x21, x22, [sp, #32]
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #48
    ret

// ---------------------------------------------------------------------------
// slow_trampoline_return_f64
//
// Zero-arg with f64 return. Shows float return path — the result comes
// back in d0 and gets passed to slow_ret_f64.
//
// fn return_f64() -> f64
// ---------------------------------------------------------------------------
.globl _slow_trampoline_return_f64
_slow_trampoline_return_f64:
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp
    str     x19, [sp, #-16]!

    mov     x19, x1                     // rv

    // --- call FFI target (no args, returns f64 in d0) ---
    adrp    x9, _ffi_target_return_f64@PAGE
    ldr     x9, [x9, _ffi_target_return_f64@PAGEOFF]
    blr     x9
    // d0 = result

    // --- set return: f64 ---
    // d0 is already in the right register for the second arg
    fmov    d1, d0                      // result
    mov     x0, x19                     // rv ptr
    adrp    x9, _slow_ret_f64@PAGE
    ldr     x9, [x9, _slow_ret_f64@PAGEOFF]
    blr     x9

    mov     w0, #1
    ldr     x19, [sp], #16
    ldp     x29, x30, [sp], #16
    ret

// ---------------------------------------------------------------------------
// Symbol stubs (would be patched with real addresses at JIT time)
// ---------------------------------------------------------------------------
.data
.align 3
.globl _ffi_target_nop, _ffi_target_add_u32, _ffi_target_hash, _ffi_target_return_f64
.globl _slow_get_i32, _slow_get_buffer, _slow_ret_u32, _slow_ret_f64
_ffi_target_nop:          .quad 0
_ffi_target_add_u32:      .quad 0
_ffi_target_hash:         .quad 0
_ffi_target_return_f64:   .quad 0
_slow_get_i32:            .quad 0
_slow_get_buffer:         .quad 0
_slow_ret_u32:            .quad 0
_slow_ret_f64:            .quad 0
