// Generates the production licence-signing keypair, once, on your own machine.
//
//   node scripts/keygen.mjs
//
// It prints the two Worker secrets and the constant that goes into the client.
// Nothing is written to disk: paste the values where they belong and close the
// terminal. If the private half ever leaks, anyone can mint Pro licences and the
// only fix is shipping a new binary with a new public key.

import { webcrypto as crypto } from "node:crypto";

const b64url = (bytes) =>
  Buffer.from(bytes).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const hex = (bytes) => Buffer.from(bytes).toString("hex");

const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const pkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
const raw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));

// Round-trip through the same import the Worker does, so a value that prints
// here is one the service will accept.
await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, false, ["sign"]);
if (raw.length !== 32) throw new Error(`public key is ${raw.length} bytes, expected 32`);

console.log("wrangler secret put SIGNING_KEY_PKCS8   <- paste this:");
console.log(b64url(pkcs8));
console.log();
console.log("wrangler secret put SIGNING_KEY_PUBLIC  <- paste this:");
console.log(hex(raw));
console.log();
console.log("src/license.rs PUBLIC_KEY_HEX           <- the same hex as above");
