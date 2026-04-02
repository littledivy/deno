const win = new Deno.BrowserWindow();

win.navigate("https://google.com");
win.addEventListener("keydown", (e) => {
  console.log("key", e.key);
  if (e.key === "p") {
    throw new Error("test");
  }
});
win.addEventListener("mousemove", (e) => {
  console.log("mouse", e.clientX, e.clientY);
});

/*

const win2 = new Deno.BrowserWindow();

win2.navigate("https://google.com");
win2.addEventListener("keydown", (e) => {
  console.log("key", e.key);
});
win2.addEventListener("mousemove", (e) => {
  console.log("mouse", e.clientX, e.clientY);
});
*/

await new Promise((res) => setTimeout(res, 100000));
