for (let i = 0;; i++) {
  console.log(i);
  Deno.statSync("README.md");
  try { Deno.statSync("file that does not exist"); } catch {}
}
