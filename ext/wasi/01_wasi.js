// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.
"use strict";

((window) => {
  const ops = Deno.core;

  class Context {
    exports;

    constructor(options) {
      this.exports = {
        proc_exit: ops.op_wasi_proc_exit.bind(ops),
      };
    }

    start(instance) {
      const { _start, _initialize, memory } = instance.exports;
      if (!(memory instanceof WebAssembly.Memory)) {
        throw new TypeError("WebAsembly.instance must provide a memory export");
      }
      if (typeof _initialize == "function") {
        throw new TypeError(
          "WebAsembly.instance export _initialize must not be a function",
        );
      }

      if (typeof _start != "function") {
        throw new TypeError(
          "WebAssembly.Instance export _start must be a function",
        );
      }

      try {
        _start();
      } catch (err) {
        if (err instanceof ExitStatus) {
          return err.code;
        }

        throw err;
      }

      return null;
    }
  }

  window.__bootstrap.wasi = {
    Context,
  };
})(this);
