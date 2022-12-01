// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

// deno-lint-ignore-file

const {
  core: {
    opAsync,
    ops,
  },
} = Deno;
const addr = Deno.args[0] || "127.0.0.1:4500";
const [hostname, port] = addr.split(":");
const serverId = ops.op_flash_serve({ hostname, port, reuseport: true });
const serverPromise = ops.op_flash_drive_server(serverId);

const fastOps = ops.op_flash_make_request();
function nextRequest() {
  return fastOps.nextRequest();
}
function respond(token, response) {
  return ops.op_flash_respond(serverId, token, response, true);
}

const response = "HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHello World";
let offset = 0;
while (true) {
  let token = nextRequest();
  if (token === 0) token = await ops.op_flash_next_async(serverId);
  for (let i = offset; i < offset + token; i++) {
    respond(
      i,
      response,
    );
  }
  offset += token;
}
