// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.
"use strict";

((window) => {
  const core = window.Deno.core;
  const ops = core.ops;
  const {
    ObjectPrototypeIsPrototypeOf,
    Uint8ArrayPrototype,
  } = window.__bootstrap.primordials;
  const { fromFlashRequest, toInnerResponse, _flash } =
    window.__bootstrap.fetch;

  function isSimpleResponse(innerResponse) {
    if (innerResponse.body !== null) {
      const responseBody = innerResponse.body.streamOrStatic?.body;
      if (
        typeof responseBody === "string" ||
        ObjectPrototypeIsPrototypeOf(Uint8ArrayPrototype, responseBody)
      ) {
        return responseBody;
      }
    }
  }

  function sendResponse(rid, response) {
    const innerResponse = toInnerResponse(response);

    // String / TypedArray
    const simpleResponse = isSimpleResponse(innerResponse);
    if (simpleResponse) {
      console.log(simpleResponse)
      writeResponse(rid, simpleResponse);
    }

    // ReadableStream
    // TODO:
  }

  function writeResponse(rid, raw) {
    const nwritten = ops.op_flash_try_write(rid, raw);
    if (nwritten < raw.byteLength) {
      ops.op_flash_write(rid, raw);
    }
  }

  const nop = () => {};

  function createServe() {
    return async function serve(callback, options) {
      await ops.op_flash_start((requestRid) => {
        const request = fromFlashRequest(0, requestRid, null, nop, nop, nop);
        const response = callback(request);
        sendResponse(requestRid, response);
      });
    };
  }

  function upgradeHttpRaw(req) {}

  window.__bootstrap.flash = {
    createServe,
    upgradeHttpRaw,
  };
})(this);
