const WS = typeof WebSocket !== "undefined" ? WebSocket : require("ws");
const websocket = new WS("ws://localhost:8000")

const msg = "hello";
function bench() {
  return new Promise((resolve) => {
    websocket.onmessage = (e) => {
      resolve();
    };
    websocket.send(msg);
  });
}

// Do a troughput benchmark.
// output: "Wrote X bytes in 1 sec, throughput: Z bytes/ms"
async function run() {
  const start = Date.now();
  let count = 0;
  let bytes = 0;
  while (Date.now() - start < 1000) {
    await bench();
    count++;
    bytes += msg.length;
  }
  console.log(`Sent`, count, `messages in 1 sec, throughput: ${bytes} bytes/ms`);
}

websocket.onopen = run;