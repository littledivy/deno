import { bench, run } from "mitata";

const hello = new TextEncoder().encode("Hello, world!");
const decoder = new TextDecoder();

bench("bench", () => {
  decoder.decode(hello)
});

run()
