// Encodings, the service's Ed25519 key, and the tokens it signs.
//
// Nothing in here knows what a payment is. It is the half of the service the
// app talks to, kept separate from the half the processors talk to so that a
// change to either cannot quietly alter the other.

import type { Env } from "./env";

export const enc = new TextEncoder();
export const now = () => Math.floor(Date.now() / 1000);

// -- encodings ---------------------------------------------------------------

export const b64url = {
  encode(bytes: Uint8Array): string {
    let s = "";
    for (const b of bytes) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  },
  decode(text: string): Uint8Array {
    const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
    return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
  },
};

/** Standard base64, which is what both processors want their MACs in. */
export const b64 = (bytes: Uint8Array): string => {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
};

export const toHex = (bytes: Uint8Array) =>
  [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");

export const fromHex = (hex: string) =>
  Uint8Array.from(hex.trim().match(/.{1,2}/g) ?? [], (b) => parseInt(b, 16));

export const randomHex = (bytes: number) =>
  toHex(crypto.getRandomValues(new Uint8Array(bytes)));

export async function sha256Hex(text: string): Promise<string> {
  return toHex(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(text))));
}

export async function hmacSha256(key: string, message: string): Promise<Uint8Array> {
  const mac = await crypto.subtle.importKey(
    "raw",
    enc.encode(key),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  return new Uint8Array(await crypto.subtle.sign("HMAC", mac, enc.encode(message)));
}

/** Compares two strings without leaking where they first differ. Both MAC
 *  checks in this service go through it: a webhook signature is exactly the
 *  kind of value an attacker gets to retry indefinitely. */
export function constantTimeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

// -- signing -----------------------------------------------------------------

/** Workers has renamed its Ed25519 identifier once already, so ask rather
 *  than assume, and ask only once per isolate. */
let algoName: string | null = null;
async function algo(): Promise<string> {
  if (algoName) return algoName;
  for (const name of ["Ed25519", "NODE-ED25519"]) {
    try {
      await crypto.subtle.generateKey({ name, namedCurve: "Ed25519" } as any, true, [
        "sign",
        "verify",
      ]);
      return (algoName = name);
    } catch {
      // try the other spelling
    }
  }
  throw new Error("this runtime has no Ed25519 in WebCrypto");
}

interface Keys {
  sign: CryptoKey;
  verify: CryptoKey;
  publicHex: string;
}

let cached: Keys | null = null;

export async function keys(env: Env): Promise<Keys> {
  if (cached) return cached;
  const name = await algo();

  let pkcs8: Uint8Array;
  let publicHex: string;

  if (env.SIGNING_KEY_PKCS8 && env.SIGNING_KEY_PUBLIC) {
    pkcs8 = b64url.decode(env.SIGNING_KEY_PKCS8);
    publicHex = env.SIGNING_KEY_PUBLIC.trim();
  } else {
    if (!env.DEV_MODE) {
      throw new Error("no signing key: set SIGNING_KEY_PKCS8 and SIGNING_KEY_PUBLIC");
    }
    // Development: mint one on first use and keep it, so the public key the
    // app was started with still verifies tomorrow's tokens.
    const row = await env.DB.prepare("SELECT pkcs8, public_hex FROM service_keys WHERE id = 1")
      .first<{ pkcs8: string; public_hex: string }>();
    if (row) {
      pkcs8 = b64url.decode(row.pkcs8);
      publicHex = row.public_hex;
    } else {
      const pair = (await crypto.subtle.generateKey(
        { name, namedCurve: "Ed25519" } as any,
        true,
        ["sign", "verify"],
      )) as CryptoKeyPair;
      pkcs8 = new Uint8Array(
        (await crypto.subtle.exportKey("pkcs8", pair.privateKey)) as ArrayBuffer,
      );
      publicHex = toHex(
        new Uint8Array((await crypto.subtle.exportKey("raw", pair.publicKey)) as ArrayBuffer),
      );
      await env.DB.prepare(
        "INSERT OR REPLACE INTO service_keys (id, pkcs8, public_hex) VALUES (1, ?, ?)",
      )
        .bind(b64url.encode(pkcs8), publicHex)
        .run();
    }
  }

  cached = {
    sign: await crypto.subtle.importKey(
      "pkcs8",
      pkcs8,
      { name, namedCurve: "Ed25519" } as any,
      false,
      ["sign"],
    ),
    verify: await crypto.subtle.importKey(
      "raw",
      fromHex(publicHex),
      { name, namedCurve: "Ed25519" } as any,
      false,
      ["verify"],
    ),
    publicHex,
  };
  return cached;
}

export interface Claims {
  lic: string;
  plan: string;
  /** Omitted rather than null when absent — the client's `Claims` skips it on
   *  serialisation too, and the signature covers the exact bytes. */
  cyc?: string;
  lim: number | null;
  dev: string;
  iat: number;
  exp: number;
}

/** `b64url(claims).b64url(sig)`, where the signature covers the ASCII bytes of
 *  the encoded payload -- not the raw JSON. The client verifies before it
 *  parses, so this order is not an implementation detail. */
export async function mint(env: Env, claims: Claims): Promise<string> {
  const { sign } = await keys(env);
  const payload = b64url.encode(enc.encode(JSON.stringify(claims)));
  const sig = new Uint8Array(
    await crypto.subtle.sign({ name: await algo() }, sign, enc.encode(payload)),
  );
  return `${payload}.${b64url.encode(sig)}`;
}

export async function open(env: Env, token: string): Promise<Claims | null> {
  const [payload, sig] = token.split(".");
  if (!payload || !sig) return null;
  const { verify } = await keys(env);
  const ok = await crypto.subtle.verify(
    { name: await algo() },
    verify,
    b64url.decode(sig),
    enc.encode(payload),
  );
  if (!ok) return null;
  try {
    return JSON.parse(new TextDecoder().decode(b64url.decode(payload))) as Claims;
  } catch {
    return null;
  }
}
