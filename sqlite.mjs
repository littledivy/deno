import { DatabaseSync } from "node:sqlite";

const db = new DatabaseSync(":memory:");
if(typeof Deno != "undefined") db.open()
db.exec("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)");
db.exec("INSERT INTO test (name) VALUES ('foo')");

const start = Date.now();
for (let i = 0; i < 1e4; i++) {
  const stmt = db.prepare("SELECT * FROM test");
  stmt.get();
}
const end = Date.now();
console.log(end - start);
