import { parsePrivateKey } from "npm:sshpk@^1.17.0";

const privateKey = Deno.readTextFileSync("example.pem");
const key = parsePrivateKey(privateKey, "pem");
const data = "example text";
const signer = key.createSign("sha256");

signer.update(data);

const signature = signer.sign();

console.log(signature);
