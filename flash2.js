const core = Deno.core;
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
  const nwritten = op_flash_try_write(rid, raw);
  if (nwritten < raw.byteLength) {
    op_flash_write(rid, raw);
  }
}

const {
  op_flash_start,
  op_flash_drive,
  op_flash_try_next,
  op_flash_next,
  op_flash_try_write,
  op_flash_write,
} = ops;

async function serve(callback, options) {
  await op_flash_start((request) => {
    
    const response = callback(request);
    writeResponse(request, response);
  });
}

const u8 = core.encode(
  "HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHello World",
);
serve(() => u8);
