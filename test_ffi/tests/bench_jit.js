// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.
// deno-lint-ignore-file

import { bench, run } from "https://esm.sh/mitata";

const targetDir = Deno.execPath().replace(/[^\/\\]+$/, "");
const [libPrefix, libSuffix] = {
  darwin: ["lib", "dylib"],
  linux: ["lib", "so"],
  windows: ["", "dll"],
}[Deno.build.os];
const libPath = `${targetDir}/${libPrefix}test_ffi.${libSuffix}`;

const dylib = Deno.dlopen(libPath, {
  "nop": { parameters: [], result: "void" },
  "nop_u8": { parameters: ["u8"], result: "void" },
  "add_u8": { parameters: ["u8", "u8"], result: "u8" },
});

bench("nop()", () => {
  dylib.symbols.nop();
});

bench("nop_u8()", () => {
  dylib.symbols.nop_u8(100);
});

bench("add_u8()", () => {
  dylib.symbols.add_u8(100, 100);
});

bench("nothing", () => {});

run()