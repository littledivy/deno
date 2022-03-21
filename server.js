async function serve(fn) {
  await Deno.core.opAsync("op_http_start_and_handle", fn);
}

await serve(({ headers }) => {
  return { status: 200, headers };
});
