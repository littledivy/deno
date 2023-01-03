// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.
const port = Bun.argv[2] || "4545";
Bun.serve({
  fetch(_req) {
    return new Response("Hello, World!");
  },
  port: Number(port),
});
