// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.
"use strict";

((window) => {
  const core = window.Deno.core;
  const ops = core.ops;
  const {
    ObjectPrototypeIsPrototypeOf,
    PromisePrototype,
    PromisePrototypeCatch,
    PromisePrototypeThen,
    Uint8ArrayPrototype,
  } = window.__bootstrap.primordials;
  const { fromFlashRequest, toInnerResponse, _flash, Response } =
    window.__bootstrap.fetch;

  const TY_STRING = 1;
  const TY_BUFFER = 2;

  // Get the type of response body.
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
      } else if (
        ObjectPrototypeIsPrototypeOf(
          ReadableStreamPrototype,
          innerResponse.body.streamOrStatic,
        )
      ) {
        if (innerResponse.body.unusable()) {
          throw new TypeError("Body is unusable.");
        }

        return TY_STREAM;
      }
    }
  }

  function sendResponse(rid, response) {
    const innerResponse = toInnerResponse(response);
    const rType = responseType(innerResponse);

    const simpleResponse = innerResponse.body.streamOrStatic?.body;
    // Static response
    if (rType === TY_STRING) {
      // TODO: Create raw HTTP response from innerResponse.
      writeResponseStr(rid, innerResponse.status ?? 200, simpleResponse);
    } else if (rType === TY_BUFFER) {
      writeResponseBytes(rid, simpleResponse);
    } else if (rType === TY_STREAM) {
      // ReadableStream
      const stream = innerResponse.body.stream;
    }
  }

  function writeResponseStr(rid, status, str) {
    const nwritten = ops.op_flash_try_write_status_str(rid, status, str);
    if (nwritten < str.length) {
      //ops.op_flash_write_str(rid, status, str);
    }
  }

    function writeResponseBytes(rid, raw) {
    const nwritten = ops.op_flash_try_write(rid, raw);
    if (nwritten < raw.byteLength) {
      ops.op_flash_write(rid, raw);
    }
  }

  const nop = () => {};
  let date_timer_running = false;

  function createServe() {
    if (!date_timer_running) {
      date_timer_running = true;
      // TODO: make this cancellable
      ops.op_flash_start_date_loop().catch(err => {
        date_timer_running = false;
      })
    }
    return async function serve(callback, options = {}) {
      const onError = options.onError ?? function (err) {
        // TODO: log the error?
        return new Response("Internal Server Error", { status: 500 });
      };
      const argsLen = callback.length;
      await ops.op_flash_start((requestRid) => {
        const request = argsLen ? fromFlashRequest(0, requestRid, null, nop, nop, nop) : undefined;
        const response = callback(request);
        if (ObjectPrototypeIsPrototypeOf(PromisePrototype, response)) {
          PromisePrototypeCatch(
            PromisePrototypeThen(
              response,
              res => {
                sendResponse(requestRid, res)
              }
            ),
            onError,
          );
          return;
        }
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
