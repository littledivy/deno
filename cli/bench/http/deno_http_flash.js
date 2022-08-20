// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

const addr = Deno.args[0] || "127.0.0.1:4500";
const [hostname, port] = addr.split(":");
const { serve } = Deno;
// import { serve } from "https://deno.land/std/http/server.ts"

async function handler(req) {
  console.log((await req.arrayBuffer()).byteLength);
  return new Response("Hello World");
}

serve(handler, {
  hostname,
  port,
});
