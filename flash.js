// const window = this;
// const { core } = Deno;
// const { print } = core;
// const { createServe } = window.__bootstrap.flash;
// const { Response } = window.__bootstrap.fetch;

// core.initializeAsyncOps();

// const serve = createServe();
const ac = new AbortController();
Deno.serve(function (req) {
  return new Response("Hello World");
}, { port: 8080, signal: ac.signal });
console.log("Listening on http://localhost:8080/");

setTimeout(() => {
  console.log("closing server");
  ac.abort();
}, 1500);
