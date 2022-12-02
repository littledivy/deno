// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.
"use strict";

((window) => {
  const core = window.Deno.core;
  const ops = core.ops;

  function sendResponse(rid, response) {
    // String / TypedArray
    if (isSimpleResponse(response)) {
      const raw = getSimpleResponse(response);
      writeResponse(rid, raw);
    }

    // ReadableStream
    // TODO:
  }

  function writeResponse(rid, raw) {
    const nwritten = ops.op_flash_try_write(rid, raw);
    if (nwritten > 0) {
      ops.op_flash_write(rid, raw);
    }
  }

  async function serve(callback, options) {
    const rid = ops.op_flash_start();
    while (true) {
      let request = ops.op_flash_try_next(rid);

      if (request === 0) {
        request = await ops.op_flash_next(rid);
      }

      const response = await callback(request);

      sendResponse(rid, response);
    }
  }

  // window.__bootstrap.flash = {
  //   serve,
  // };
})(this);
