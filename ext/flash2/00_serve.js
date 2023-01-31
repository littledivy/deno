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

  function hostnameForDisplay(hostname) {
    // If the hostname is "0.0.0.0", we display "localhost" in console
    // because browsers in Windows don't resolve "0.0.0.0".
    // See the discussion in https://github.com/denoland/deno_std/issues/1165
    return hostname === "0.0.0.0" ? "localhost" : hostname;
  }

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
  let dateTimerRunning = false;

  function startDateLoop() {
    if (!dateTimerRunning) {
      dateTimerRunning = true;
      ops.op_flash_start_date_loop().catch((err) => {
        dateTimerRunning = false;
      });
    }
  }

  function stopDateLoop() {
    if (dateTimerRunning) {
      ops.op_flash_stop_date_loop();
      dateTimerRunning = false;
    }
  }

  function argsToOptions(arg1, arg2) {
    let options = undefined;
    let handler = undefined;
    if (typeof arg1 === "function") {
      handler = arg1;
      options = arg2;
    } else if (typeof arg2 === "function") {
      handler = arg2;
      options = arg1;
    } else {
      options = arg1;
    }
    if (handler === undefined) {
      if (options === undefined) {
        throw new TypeError(
          "No handler was provided, so an options bag is mandatory.",
        );
      }
      handler = options.handler;
    }
    if (typeof handler !== "function") {
      throw new TypeError("A handler function must be provided.");
    }
    if (options === undefined) {
      options = {};
    }

    return { options, handler };
  }

  function createServe() {
    return async function serve(arg1, arg2) {
      const { options, handler } = argsToOptions(arg1, arg2);
      const onError = options.onError ?? function (err) {
        console.error(err);
        return new Response("Internal Server Error", { status: 500 });
      };

      const onListen = options.onListen ?? function ({ port }) {
        console.log(
          `Listening on http://${
            hostnameForDisplay(listenOpts.hostname)
          }:${port}/`,
        );
      };

      const listenOpts = {
        hostname: options.hostname ?? "127.0.0.1",
        port: options.port ?? 4500,
        reuseport: options.reusePort ?? false,
      };

      if (options.cert || options.key) {
        if (!options.cert || !options.key) {
          throw new TypeError(
            "Both cert and key must be provided to enable HTTPS.",
          );
        }
        listenOpts.cert = options.cert;
        listenOpts.key = options.key;
      }

      const signal = options.signal;

      const argsLen = handler.length;

      const [serverId, port] = ops.op_flash_start((requestRid) => {
        const request = argsLen
          ? fromFlashRequest(
            0,
            requestRid,
            ops.op_flash_get_has_body(requestRid)
              ? createRequestBodyStream(requestRid)
              : null,
            () => ops.op_flash_get_method(requestRid),
            () => ops.op_flash_get_url(requestRid),
            () => ops.op_flash_get_headers(requestRid),
          )
          : undefined;

        const response = handler(request);

        if (
          typeof response.then == "function" ||
          ObjectPrototypeIsPrototypeOf(PromisePrototype, response)
        ) {
          PromisePrototypeCatch(
            PromisePrototypeThen(
              response,
              (res) => {
                sendResponse(requestRid, res);
              },
            ),
            onError,
          );
        } else {
          sendResponse(requestRid, response);
        }
      }, listenOpts);

      signal?.addEventListener("abort", () => {
        stopDateLoop();
        PromisePrototypeThen(server.close(), () => {}, () => {});
      }, {
        once: true,
      });

      onListen({ hostname: listenOpts.hostname, port });
      const serverPromise = ops.op_flash_drive(serverId);
      const finishedPromise = PromisePrototypeCatch(serverPromise, () => {});

      const server = {
        transport: listenOpts.cert && listenOpts.key ? "https" : "http",
        hostname: listenOpts.hostname,
        port: listenOpts.port,
        closed: false,
        finished: finishedPromise,
        async close() {
          if (server.closed) {
            return;
          }
          server.closed = true;
          ops.op_flash_close(serverId);
          await server.finished;
        },
      };

      try {
        await serverPromise;
      } catch (err) {
        console.error(err);
      }
    };
  }

  function createRequestBodyStream(requestId) {
    // The first packet is left over bytes after parsing the request
    const firstRead = ops.op_flash_first_packet(requestId);
    if (!firstRead) return null;
    let firstEnqueued = firstRead.byteLength == 0;

    return new ReadableStream({
      type: "bytes",
      async pull(controller) {
        try {
          if (firstEnqueued === false) {
            controller.enqueue(firstRead);
            firstEnqueued = true;
            return;
          }
          // This is the largest possible size for a single packet on a TLS
          // stream.
          const chunk = new Uint8Array(16 * 1024 + 256);
          const read = await ops.op_flash_read_body(requestId, chunk);
          if (read > 0) {
            // We read some data. Enqueue it onto the stream.
            controller.enqueue(TypedArrayPrototypeSubarray(chunk, 0, read));
          } else {
            // We have reached the end of the body, so we close the stream.
            controller.close();
          }
        } catch (err) {
          // There was an error while reading a chunk of the body, so we
          // error.
          controller.error(err);
          controller.close();
        }
      },
    });
  }

  function upgradeHttpRaw(req) {}

  window.__bootstrap.flash = {
    createServe,
    upgradeHttpRaw,
  };
})(this);
