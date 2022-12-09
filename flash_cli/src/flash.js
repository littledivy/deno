const { ops, initializeAsyncOps } = Deno.core;

initializeAsyncOps();

const {
  op_flash_try_write_str,
  op_flash_start,
  op_flash_try_write_status_str,
  op_flash_try_write,
  op_flash_write,
  op_flash_start_date_loop,
} = ops;

const Types = {
  Default: 1,
  Buffer: 2,
};

class Response {
  status = 200;
  contentType = "text/plain;charset=UTF-8";
  statusMessage = "";
  type = Types.Default;
  body = null;
  headerList = [];
  urlList = [];

  constructor(body = null, init = undefined) {
    if (typeof body === "string") {
      this.body = body;
      return;
    }
    if (body.constructor.name === "Uint8Array") {
      this.body = body;
      this.type = Types.Buffer;
      return;
    }
  }

  url() {
    if (this.urlList.length == 0) return null;
    return this.urlList[this.urlList.length - 1];
  }
}

function createResponseString(res) {
  const { status, statusMessage, body, contentType } = res;
  return `HTTP/1.1 ${status} ${statusMessage} \r\nDate: ${now}\r\nContent-Length: ${13}\r\nContent-Type: ${contentType}\r\n\r\n${body}`;
}

function sendResponse(rid, res) {
  if (res.type === Types.Default) {
    op_flash_try_write_status_str(rid, res.status, res.body);
  } else if (res.type === Types.Buffer) {
    const nwritten = op_flash_try_write(rid, res.body);
    if (nwritten < res.body.byteLength) {
      op_flash_write(rid, res.body);
    }
  }
}

Deno.serve = async (fetch, options) => {
  const isAsync = fetch.constructor.name === "AsyncFunction" ? true : false;
  const argLen = fetch.length;

  if (!isAsync) {
    if (argLen === 0) {
      await op_flash_start((rid) => {
        const res = fetch();
        sendResponse(rid, res);
      });
    } else {
      await op_flash_start((rid) => {
        const request = fromFlashRequest(0, rid, null, nop, nop, nop);
        const res = fetch(request);
        sendResponse(rid, res);
      });
    }
  } else {
    if (argLen === 0) {
      await op_flash_start(async (rid) => {
        const res = await fetch();
        sendResponse(rid, res);
      });
    } else {
      await op_flash_start(async (rid) => {
        const request = fromFlashRequest(0, rid, null, nop, nop, nop);
        const res = await fetch(request);
        sendResponse(rid, res);
      });
    }
  }
};

op_flash_start_date_loop();

Deno.serve(() => new Response("Hello, World!"));
