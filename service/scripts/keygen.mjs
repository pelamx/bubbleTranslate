// Generates the licence service's Ed25519 signing keypair.
//
// Run this once, ever. The private half signs every entitlement token the app
// trusts; if it leaks, anyone can mint Pro and the only fix is shipping a new
// binary, because the public half is compiled into `src/license.rs`.
//
// The private key is never printed. It goes straight to a 0600 file that git
// ignores, and the secret is set by piping that file — so it never reaches the
// terminal, the scrollback, or shell history. Only the public half, which is
// meant to be published, is shown.
//
//   node scripts/keygen.mjs
//
// Then, exactly as written — the argument is the secret's NAME, and the value
// is read from the file on stdin:
//
//   npx wrangler secret put SIGNING_KEY_PKCS8  < .keys/signing_key_pkcs8
//   npx wrangler secret put SIGNING_KEY_PUBLIC < .keys/signing_key_public

import { webcrypto as crypto } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const b64url = (bytes) =>
  Buffer.from(bytes).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);

const pkcs8 = b64url(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
const publicHex = Buffer.from(await crypto.subtle.exportKey("raw", pair.publicKey)).toString("hex");

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", ".keys");
mkdirSync(dir, { recursive: true, mode: 0o700 });
// No trailing newline: `wrangler secret put < file` sends the bytes verbatim,
// and a stray \n would be part of the secret.
writeFileSync(join(dir, "signing_key_pkcs8"), pkcs8, { mode: 0o600 });
writeFileSync(join(dir, "signing_key_public"), publicHex, { mode: 0o600 });

console.log(`
Keypair written to service/.keys/ (gitignored, 0600). The private half was not
printed — keep it that way.

  1. Set both secrets. The argument is the NAME; the value comes from the file:

     npx wrangler secret put SIGNING_KEY_PKCS8  < .keys/signing_key_pkcs8
     npx wrangler secret put SIGNING_KEY_PUBLIC < .keys/signing_key_public

  2. Put this public key in PUBLIC_KEY_HEX in src/license.rs, then rebuild:

     ${publicHex}

  3. Redeploy:  npx wrangler deploy

Every licence ever issued is verified against that hex. Changing it later
invalidates every token in the field, so it is a one-time decision.
`);
