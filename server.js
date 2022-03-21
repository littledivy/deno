await Deno.serveHttp(({ respondWith }) => {
  respondWith(new Response("Hello, World"));
});
