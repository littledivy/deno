const window = this;
const { core } = Deno;
const { print } = core;
const { createServe } = window.__bootstrap.flash;
const { Response } = window.__bootstrap.fetch;

core.initializeAsyncOps();

const serve = createServe();
serve(function (req) {
  return new Response("Hello World");
}, { port: 8080 });
