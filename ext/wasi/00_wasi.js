// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

/// <reference path="../../core/internal.d.ts" />

"use strict";

((window) => {
  const core = window.Deno.core;
  const ops = core.ops;

  class Context {
    #rid;
    exports;
    #started;

    constructor(options = {}) {
      this.#started = false;
      this.#rid = ops.op_wasi_create();
      this.exports = {};
    }

    start(instance) {
      if (this.#started) {
        throw new Error("WebAssembly.Instance has already started");
      }

      this.#started = true;

      const { _start, _initialize, memory } = instance.exports;

      if (!(memory instanceof WebAssembly.Memory)) {
        throw new TypeError("WebAsembly.instance must provide a memory export");
      }

      ops.op_wasi_set_memory(this.#rid, memory);

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

      _start();

      return null;
    }
  }
})(this);
