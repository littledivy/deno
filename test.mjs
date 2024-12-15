import { WASI } from "node:wasi";
import fs from "node:fs";

const wasi = new WASI({
  args: [],
  returnOnExit: true,
  stdin: 0,
  stdout: 1,
  stderr: 2,
  version: "preview1",
});

console.log(wasi.wasiImports());
const importObject = { wasi_snapshot_preview1: wasi.wasiImports() };

const wasm = fs.readFileSync("exitcode.wasm");

WebAssembly.instantiate(wasm, importObject).then((result) => {
  wasi.start(result.instance);
});
