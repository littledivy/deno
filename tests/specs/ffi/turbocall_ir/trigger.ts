const p = Deno.execPath().replace(/[^\/\\]+$/, "");
const [libPrefix, libSuffix] = {
  darwin: ["lib", "dylib"],
  linux: ["lib", "so"],
  windows: ["", "dll"],
}[Deno.build.os]!;

Deno.dlopen(`${p}${libPrefix}test_ffi.${libSuffix}`, {
  nop: { parameters: [], result: "void" },
  add_u32: { parameters: ["u32", "u32"], result: "u32" },
  nop_bool: { parameters: ["bool"], result: "void" },
  nop_u8: { parameters: ["u8"], result: "void" },
  nop_i32: { parameters: ["i32"], result: "void" },
  nop_f64: { parameters: ["f64"], result: "void" },
  nop_buffer: { parameters: ["buffer"], result: "void" },
  return_bool: { parameters: [], result: "bool" },
  return_u32: { parameters: [], result: "u32" },
  return_i32: { parameters: [], result: "i32" },
  return_f64: { parameters: [], result: "f64" },
  hash: { parameters: ["buffer", "u32"], result: "u32" },
});
