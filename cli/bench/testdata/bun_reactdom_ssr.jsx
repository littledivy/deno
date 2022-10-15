// Bun uses a custom non-portable react-dom fork.
// TODO(@littledivy): Reenable this when it stops segfaulting.
import { renderToReadableStream } from "./react-dom.js";

const App = () => (
  <html>
    <body>
      <h1>Hello World</h1>
    </body>
  </html>
);

Bun.serve({
  async fetch(req) {
    // return new Response(new ReadableStream({
    //   type: "direct",
    //   start(controller) {
    //     controller.write("<html><body><h1>Hello World</h1></body></html>");
    //     controller.close();
    //   },
    //   close() {},
    // }));
    return new Response(await renderToReadableStream(<App />), {
      headers: {
        "Content-Type": "text/html",
        "Date": (new Date()).toUTCString(),
      },
    });
  },
  port: 9000,
});
