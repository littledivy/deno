// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.
const core = globalThis.Deno.core;
const primordials = globalThis.__bootstrap.primordials;

const { BadResourcePrototype } = core;
import { InnerBody } from "ext:deno_fetch/22_body.js";
import { Event } from "ext:deno_web/02_event.js";
import { toInnerResponse } from "ext:deno_fetch/23_response.js";
import { fromInnerRequest } from "ext:deno_fetch/23_request.js";
import { AbortController } from "ext:deno_web/03_abort_signal.js";
import {
  _eventLoop,
  _idleTimeoutDuration,
  _idleTimeoutTimeout,
  _protocol,
  _readyState,
  _rid,
  _role,
  _server,
  _serverHandleIdleTimeout,
  SERVER,
  WebSocket,
} from "ext:deno_websocket/01_websocket.js";
import {
  getReadableStreamResourceBacking,
  readableStreamForRid,
  ReadableStreamPrototype,
  writableStreamForRid,
} from "ext:deno_web/06_streams.js";
const {
  ObjectPrototypeIsPrototypeOf,
  SafeSet,
  SafeSetIterator,
  SetPrototypeAdd,
  SetPrototypeDelete,
  Symbol,
  TypeError,
  Uint8ArrayPrototype,
} = primordials;

// Symbol we use to communicate to the rest of Deno that we handle upgrades ourselves
const _wantsUpgrade = Symbol("_wantsUpgrade");
const _upgraded = Symbol("_upgraded");

class RequestHeaders {
  #slabId;

  get(header) {
    const value = core.ops.op_get_request_header(this.#slabId, header);
    return value;
  }
}

class InnerRequest {
  #slabId;
  #headers;
  #context;
  #methodAndUri;
  #streamRid;
  #body;
  #upgraded;

  constructor(slabId, context) {
    this.#slabId = slabId;
    this.#context = context;
    this.#upgraded = false;
  }

  close() {
    if (this.#streamRid !== undefined) {
      core.close(this.#streamRid);
      this.#streamRid = undefined;
    }
    this.#slabId = undefined;
  }

  get [_upgraded]() {
    return this.#upgraded;
  }

  [_wantsUpgrade](upgradeType, ...originalArgs) {
    this.#upgraded = true;

    // upgradeHttp is async
    // TODO(mmastrac)
    if (upgradeType == "upgradeHttp") {
      throw "upgradeHttp is unavailable in Deno.serve at this time";
    }

    // upgradeHttpRaw is async
    // TODO(mmastrac)
    if (upgradeType == "upgradeHttpRaw") {
      throw "upgradeHttp is unavailable in Deno.serve at this time";
    }

    // upgradeWebSocket is sync
    if (upgradeType == "upgradeWebSocket") {
      const response = originalArgs[0];
      const ws = originalArgs[1];

      this.url();
      this.headerList;
      this.close();

      // Start the upgrade in the background.
      (async () => {
        try {
          const upgrade = await core.opAsync2(
            "op_upgrade",
            this.#slabId,
            response.headerList,
          );
          const wsRid = core.ops.op_ws_server_create(upgrade);
          ws[_rid] = wsRid;
          ws[_readyState] = WebSocket.OPEN;
          ws[_role] = SERVER;
          const event = new Event("open");
          ws.dispatchEvent(event);

          ws[_eventLoop]();
          if (ws[_idleTimeoutDuration]) {
            ws.addEventListener(
              "close",
              () => clearTimeout(ws[_idleTimeoutTimeout]),
            );
          }
          ws[_serverHandleIdleTimeout]();
        } catch (error) {
          const event = new ErrorEvent("error", { error });
          ws.dispatchEvent(event);
        }
      })();
      return { response: new Response(), socket: ws };
    }
  }

  url() {
    if (this.#methodAndUri === undefined) {
      if (this.#slabId === undefined) {
        throw new TypeError("request closed");
      }
      this.#methodAndUri = core.ops.op_get_request_method_and_url(this.#slabId);
    }
    if (this.#methodAndUri[1]) {
      return this.#context.scheme + this.#methodAndUri[1] +
        this.#methodAndUri[2];
    }
    return this.#context.scheme + this.#methodAndUri[2];
  }

  get remoteAddr() {
    if (this.#methodAndUri === undefined) {
      if (this.#slabId === undefined) {
        throw new TypeError("request closed");
      }
      this.#methodAndUri = core.ops.op_get_request_method_and_url(this.#slabId);
    }
    return {
      transport: "tcp",
      hostname: this.#methodAndUri[3],
      port: this.#methodAndUri[4],
    };
  }

  get method() {
    if (this.#methodAndUri === undefined) {
      if (this.#slabId === undefined) {
        throw new TypeError("request closed");
      }
      this.#methodAndUri = core.ops.op_get_request_method_and_url(this.#slabId);
    }
    return this.#methodAndUri[0];
  }

  get body() {
    if (this.#slabId === undefined) {
      throw new TypeError("request closed");
    }
    if (this.#body !== undefined) {
      return this.#body;
    }
    // If the method is GET or HEAD, we do not want to include a body here, even if the Rust
    // side of the code is willing to provide it to us.
    if (this.method == "GET" || this.method == "HEAD") {
      this.#body = null;
      return null;
    }
    this.#streamRid = core.ops.op_read_request_body(this.#slabId);
    this.#body = new InnerBody(readableStreamForRid(this.#streamRid, false));
    return this.#body;
  }

  get headerList() {
    if (this.#slabId === undefined) {
      throw new TypeError("request closed");
    }
    return core.ops.op_get_request_headers(this.#slabId);
  }

  get headers() {
    if (!this.#headers) {
      if (this.#slabId === undefined) {
        throw new TypeError("request closed");
      }
      this.#headers = new RequestHeaders(this.#slabId);
    }
    return this.#headers;
  }

  get slabId() {
    return this.#slabId;
  }
}

class CallbackContext {
  scheme;
  fallbackHost;
  serverRid;

  initialize(scheme, fallbackHost) {
    this.scheme = scheme;
    this.fallbackHost = fallbackHost;
  }
}

function fastSyncResponseOrStream(req, respBody) {
  if (respBody === null || respBody === undefined) {
    // Don't set the body
    return null;
  }

  const stream = respBody.streamOrStatic;
  const body = stream.body;

  if (ObjectPrototypeIsPrototypeOf(Uint8ArrayPrototype, body)) {
    core.ops.op_set_response_body_bytes(req, body);
    return null;
  }

  if (typeof body === "string") {
    core.ops.op_set_response_body_text(req, body);
    return null;
  }

  // At this point in the response it needs to be a stream
  if (!ObjectPrototypeIsPrototypeOf(ReadableStreamPrototype, stream)) {
    throw TypeError("invalid response");
  }
  const resourceBacking = getReadableStreamResourceBacking(stream);
  if (resourceBacking) {
    core.ops.op_set_response_body_resource(
      req,
      resourceBacking.rid,
      resourceBacking.autoClose,
    );
    return null;
  }

  return stream;
}

async function asyncResponse(responseBodies, req, status, stream) {
  const responseRid = core.ops.op_set_response_body_stream(req);
  SetPrototypeAdd(responseBodies, responseRid);
  const reader = stream.getReader();
  const responseStream = writableStreamForRid(responseRid, true);
  const writer = responseStream.getWriter();
  core.ops.op_set_promise_complete(req, status);
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      await writer.write(value);
    }
  } catch (error) {
    await reader.cancel(error);
  } finally {
    SetPrototypeDelete(responseBodies, responseRid);
    writer.releaseLock();
    reader.releaseLock();
  }
  responseStream.close();
}

/**
 * Maps the incoming request slab ID to a fully-fledged Request object, passes it to the user-provided
 * callback, then extracts the response that was returned from that callback. The response is then pulled
 * apart and handled on the Rust side.
 */
function mapToCallback(responseBodies, context, signal, callback, onError) {
  return function (req) {
    const innerRequest = new InnerRequest(req, context);
    const request = fromInnerRequest(innerRequest, signal, "immutable");
    let response;
    try {
      response = callback(request, { remoteAddr: innerRequest.remoteAddr });
    } catch (error) {
      response = onError(error);
    }
    return (async () => {
      try {
        const inner = toInnerResponse(await response);
        if (innerRequest[_upgraded]) {
          // We're done here as the connection has been upgraded during the callback and no longer requires servicing.
          return;
        }

        // Did everything shut down while we were waiting?
        if (context.closed) {
          innerRequest.close();
          return;
        }

        const status = inner.status;
        const headers = inner.headerList;
        if (headers && headers.length > 0) {
          if (headers.length == 1) {
            core.ops.op_set_response_header(req, headers[0][0], headers[0][1]);
          } else {
            core.ops.op_set_response_headers(req, headers);
          }
        }

        // Attempt to response quickly to this request, otherwise extract the stream
        const stream = fastSyncResponseOrStream(req, inner.body);
        if (stream !== null) {
          // Handle the stream asynchronously
          await asyncResponse(responseBodies, req, status, stream);
        } else {
          core.ops.op_set_promise_complete(req, status);
        }

        innerRequest.close();
      } catch (e) {
        onError(e);
      }
    })();
  };
}

async function serve(arg1, arg2) {
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

  const signal = options.signal;
  const onError = options.onError ?? function (error) {
    console.error(error);
    return new Response("Internal Server Error", { status: 500 });
  };
  const listenOpts = {
    hostname: options.hostname ?? "0.0.0.0",
    port: options.port ?? 9000,
    reusePort: options.reusePort ?? false,
  };

  const abortController = new AbortController();

  const responseBodies = new SafeSet();
  const context = new CallbackContext();
  const callback = mapToCallback(
    responseBodies,
    context,
    abortController.signal,
    handler,
    onError,
  );

  let rid;
  if (options.cert || options.key) {
    if (!options.cert || !options.key) {
      throw new TypeError(
        "Both cert and key must be provided to enable HTTPS.",
      );
    }
    listenOpts.cert = options.cert;
    listenOpts.key = options.key;
    listenOpts.alpnProtocols = ["h2", "http/1.1"];
    const listener = Deno.listenTls(listenOpts);
    listenOpts.port = listener.addr.port;
    rid = core.ops.op_serve_http(
      listener.rid,
      context.initialize.bind(context),
    );
    context.serverRid = rid;
  } else {
    const listener = Deno.listen(listenOpts);
    listenOpts.port = listener.addr.port;
    rid = core.ops.op_serve_http(
      listener.rid,
      context.initialize.bind(context),
    );
    context.serverRid = rid;
  }

  signal?.addEventListener(
    "abort",
    () => {
      try {
        context.closed = true;
        core.tryClose(rid);
      } catch {
        // Pass
      }
    },
    { once: true },
  );

  const onListen = options.onListen ?? function ({ port }) {
    // If the hostname is "0.0.0.0", we display "localhost" in console
    // because browsers in Windows don't resolve "0.0.0.0".
    // See the discussion in https://github.com/denoland/deno_std/issues/1165
    const hostname = listenOpts.hostname == "0.0.0.0"
      ? "localhost"
      : listenOpts.hostname;
    console.log(`Listening on ${context.scheme}${hostname}:${port}/`);
  };

  onListen({ port: listenOpts.port });

  while (true) {
    let req;
    try {
      req = await core.opAsync("op_http_wait", rid);
    } catch (error) {
      if (ObjectPrototypeIsPrototypeOf(BadResourcePrototype, error)) {
        break;
      }
      throw new Deno.errors.Http(error);
    }
    if (req === 0xffffffff) {
      break;
    }
    callback(req);
  }

  for (const streamRid of new SafeSetIterator(responseBodies)) {
    core.tryClose(streamRid);
  }
}

export { _wantsUpgrade, serve };
