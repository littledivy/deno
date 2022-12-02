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

  const TY_STRING = 1;
  const TY_BUFFER = 2;

  function responseType(innerResponse) {
    if (innerResponse.body !== null) {
      const responseBody = innerResponse.body.streamOrStatic?.body;
      if (
        typeof responseBody === "string"
      ) {
        return TY_STRING;
      } else if (
        ObjectPrototypeIsPrototypeOf(Uint8ArrayPrototype, responseBody)
      ) {
        return TY_BUFFER;
      } else if (ObjectPrototypeIsPrototypeOf(
        ReadableStreamPrototype,
        innerResponse.body.streamOrStatic,
      )) {
        if (innerResponse.body.unusable()) {
          throw new TypeError("Body is unusable.");
        }

        return TY_STREAM;
      }
    }
  }

  function sendResponse(rid, response) {
    const innerResponse = toInnerResponse(response);
    const responseType = responseType(innerResponse);

    const simpleResponse = innerResponse.body.streamOrStatic?.body;
    // Static response
    if (responseType === TY_STRING) {
      writeResponseStr(rid, simpleResponse);
    } else if (responseType === TY_BUFFER) {
      writeResponseBytes(rid, simpleResponse);
    } else if (responseType === TY_STREAM) {
      // ReadableStream
      const stream = innerResponse.body.stream;
    }    
  }

  function writeResponseStr(rid, raw) {
    const nwritten = ops.op_flash_try_write_str(rid, raw);
    if (nwritten < raw.length) {
      ops.op_flash_write_str(rid, raw);
    }
  }

  function writeResponseBytes(rid, raw) {
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
