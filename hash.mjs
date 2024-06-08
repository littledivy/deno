import { createHash } from "node:crypto";

for (let i = 0; i < 1e7; i++) {
  const hash = createHash("sha256");
  hash.update("hello world");
}
