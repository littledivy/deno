import crypto from "node:crypto";

// const bob = crypto.createDiffieHellman(512);
// const bobKey = bob.generateKeys();
// const alice = crypto.createDiffieHellman(bob.getPrime(), bob.getGenerator());
// const aliceKey = alice.generateKeys();
// console.log(bob.computeSecret(aliceKey).toString('hex'));

const privateKey = crypto.createPrivateKey({
  key: "-----BEGIN PRIVATE KEY-----\n" +
    "MIIBoQIBADCB1QYJKoZIhvcNAQMBMIHHAoHBAP//////////yQ/aoiFowjTExmKL\n" +
    "gNwc0SkCTgiKZ8x0Agu+pjsTmyJRSgh5jjQE3e+VGbPNOkMbMCsKbfJfFDdP4TVt\n" +
    "bVHCReSFtXZiXn7G9ExC6aY37WsL/1y29Aa37e44a/taiZ+lrp8kEXxLH+ZJKGZR\n" +
    "7ORbPcIAfLihY78FmNpINhxV05ppFj+o/STPX4NlXSPco62WHGLzViCFUrue1SkH\n" +
    "cJaWbWcMNU5KvJgE8XRsCMojcyf//////////wIBAgSBwwKBwEh82IAVnYNf0Kjb\n" +
    "qYSImDFyg9sH6CJ0GzRK05e6hM3dOSClFYi4kbA7Pr7zyfdn2SH6wSlNS14Jyrtt\n" +
    "HePrRSeYl1T+tk0AfrvaLmyM56F+9B3jwt/nzqr5YxmfVdXb2aQV53VS/mm3pB2H\n" +
    "iIt9FmvFaaOVe2DupqSr6xzbf/zyON+WF5B5HNVOWXswgpgdUsCyygs98hKy/Xje\n" +
    "TGzJUoWInW39t0YgMXenJrkS0m6wol8Rhxx81AGgELNV7EHZqg==\n" +
    "-----END PRIVATE KEY-----",
  format: "pem",
});

const publicKey = crypto.createPublicKey({
  key: "-----BEGIN PUBLIC KEY-----\n" +
    "MIIBoDCB1QYJKoZIhvcNAQMBMIHHAoHBAP//////////yQ/aoiFowjTExmKLgNwc\n" +
    "0SkCTgiKZ8x0Agu+pjsTmyJRSgh5jjQE3e+VGbPNOkMbMCsKbfJfFDdP4TVtbVHC\n" +
    "ReSFtXZiXn7G9ExC6aY37WsL/1y29Aa37e44a/taiZ+lrp8kEXxLH+ZJKGZR7ORb\n" +
    "PcIAfLihY78FmNpINhxV05ppFj+o/STPX4NlXSPco62WHGLzViCFUrue1SkHcJaW\n" +
    "bWcMNU5KvJgE8XRsCMojcyf//////////wIBAgOBxQACgcEAi26oq8z/GNSBm3zi\n" +
    "gNt7SA7cArUBbTxINa9iLYWp6bxrvCKwDQwISN36/QUw8nUAe8aRyMt0oYn+y6vW\n" +
    "Pw5OlO+TLrUelMVFaADEzoYomH0zVGb0sW4aBN8haC0mbrPt9QshgCvjr1hEPEna\n" +
    "QFKfjzNaJRNMFFd4f2Dn8MSB4yu1xpA1T2i0JSk24vS2H55jx24xhUYtfhT2LJgK\n" +
    "JvnaODey/xtY4Kql10ZKf43Lw6gdQC3G8opC9OxVxt9oNR7Z\n" +
    "-----END PUBLIC KEY-----",
  format: "pem",
});

const secret = crypto.diffieHellman({
  privateKey,
  publicKey,
});

console.log(secret.toString("hex"));
