// The licence service: three endpoints the app calls, and one the payment
// processor calls.
//
// It is deliberately not on the path of a translation and never sees one.
// All it does is decide whether a licence key still entitles a machine to
// have its counter lifted, and say so in a token the app can check on its
// own for the next thirty days. If this service is down, every install
// degrades to the free tier rather than to a broken app.

export interface Env {
  DB: D1Database;
  /** Opens the dev-only routes and lets the service keep its own key. */
  DEV_MODE?: string;
  /** Production signing key: base64url PKCS#8, set with `wrangler secret put`. */
  SIGNING_KEY_PKCS8?: string;
  /** Its public half, 32 bytes as hex. This is what goes in PUBLIC_KEY_HEX. */
  SIGNING_KEY_PUBLIC?: string;
  LEMONSQUEEZY_WEBHOOK_SECRET?: string;
}

/** Matches the client's TOKEN_TTL_HINT. Long enough that a service outage is
 *  invisible to a paying user, short enough that a refund lapses on its own. */
const TOKEN_TTL = 30 * 86_400;
const DEFAULT_SEATS = 3;

const enc = new TextEncoder();
const now = () => Math.floor(Date.now() / 1000);

// -- encodings ---------------------------------------------------------------

const b64url = {
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

const toHex = (bytes: Uint8Array) =>
  [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");

const fromHex = (hex: string) =>
  Uint8Array.from(hex.trim().match(/.{1,2}/g) ?? [], (b) => parseInt(b, 16));

async function sha256Hex(text: string): Promise<string> {
  return toHex(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(text))));
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

async function keys(env: Env): Promise<Keys> {
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
      pkcs8 = new Uint8Array((await crypto.subtle.exportKey("pkcs8", pair.privateKey)) as ArrayBuffer);
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
    sign: await crypto.subtle.importKey("pkcs8", pkcs8, { name, namedCurve: "Ed25519" } as any, false, [
      "sign",
    ]),
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

interface Claims {
  lic: string;
  plan: string;
  lim: number | null;
  dev: string;
  iat: number;
  exp: number;
}

/** `b64url(claims).b64url(sig)`, where the signature covers the ASCII bytes of
 *  the encoded payload -- not the raw JSON. The client verifies before it
 *  parses, so this order is not an implementation detail. */
async function mint(env: Env, claims: Claims): Promise<string> {
  const { sign } = await keys(env);
  const payload = b64url.encode(enc.encode(JSON.stringify(claims)));
  const sig = new Uint8Array(
    await crypto.subtle.sign({ name: await algo() }, sign, enc.encode(payload)),
  );
  return `${payload}.${b64url.encode(sig)}`;
}

async function open(env: Env, token: string): Promise<Claims | null> {
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

// -- licence keys ------------------------------------------------------------

/** No I, L, O, 0 or 1: this gets read off an email and typed into a text box. */
const ALPHABET = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";

function newLicenceKey(): string {
  const out: string[] = [];
  while (out.length < 15) {
    for (const b of crypto.getRandomValues(new Uint8Array(24))) {
      // Rejection sampling: 248 is the largest multiple of 31 under 256, so
      // every letter stays equally likely.
      if (b < 248 && out.length < 15) out.push(ALPHABET[b % ALPHABET.length]);
    }
  }
  const g = out.join("");
  return `BT-${g.slice(0, 5)}-${g.slice(5, 10)}-${g.slice(10, 15)}`;
}

// -- replies -----------------------------------------------------------------

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

/** The client shows this string to the user verbatim, so it is a sentence
 *  addressed to a person, not a code addressed to a developer. */
const refuse = (message: string, status = 400) => json({ error: message }, status);

interface Licence {
  id: string;
  plan: string;
  daily_limit: number | null;
  status: string;
  seat_limit: number;
  renews_at: string | null;
}

async function grant(env: Env, licence: Licence, device: string) {
  const issued = now();
  const token = await mint(env, {
    lic: licence.id,
    plan: licence.plan,
    lim: licence.daily_limit,
    dev: device,
    iat: issued,
    exp: issued + TOKEN_TTL,
  });
  return json({ token, renews: licence.renews_at });
}

// -- routes ------------------------------------------------------------------

async function activate(env: Env, body: any) {
  const key = String(body.key ?? "").trim().toUpperCase();
  const device = String(body.device ?? "").trim();
  if (!key) return refuse("Enter your licence key first.");
  if (!device) return refuse("This copy could not identify the machine it is running on.");

  const licence = await env.DB.prepare(
    "SELECT id, plan, daily_limit, status, seat_limit, renews_at FROM licences WHERE key_hash = ?",
  )
    .bind(await sha256Hex(key))
    .first<Licence>();

  if (!licence) return refuse("That licence key was not recognised. Check it for typos.", 404);
  if (licence.status !== "active") {
    return refuse("This licence is no longer active. Contact support if that is unexpected.", 403);
  }

  const seen = now();
  const held = await env.DB.prepare("SELECT device FROM seats WHERE licence_id = ? AND device = ?")
    .bind(licence.id, device)
    .first();

  if (held) {
    // Re-activating on a machine that already has a seat is not a new seat.
    await env.DB.prepare("UPDATE seats SET last_seen = ? WHERE licence_id = ? AND device = ?")
      .bind(seen, licence.id, device)
      .run();
  } else {
    const { count } = (await env.DB.prepare(
      "SELECT COUNT(*) AS count FROM seats WHERE licence_id = ?",
    )
      .bind(licence.id)
      .first<{ count: number }>())!;
    if (count >= licence.seat_limit) {
      return refuse(
        `This licence is already in use on ${licence.seat_limit} machines. ` +
          "Open the Account tab on one of them and choose Deactivate, then try again.",
        409,
      );
    }
    await env.DB.prepare(
      "INSERT INTO seats (licence_id, device, os, app, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?)",
    )
      .bind(licence.id, device, String(body.os ?? ""), String(body.app ?? ""), seen, seen)
      .run();
  }

  return grant(env, licence, device);
}

async function refresh(env: Env, body: any) {
  const claims = await open(env, String(body.token ?? ""));
  if (!claims) return refuse("This licence could not be checked. Enter your key again.", 401);

  const device = String(body.device ?? "").trim();
  // A token is a bearer credential for exactly one machine. Refreshing one
  // issued to a different device would turn it into a transferable one.
  if (device !== claims.dev) {
    return refuse("This licence was issued to a different machine.", 403);
  }

  const licence = await env.DB.prepare(
    "SELECT id, plan, daily_limit, status, seat_limit, renews_at FROM licences WHERE id = ?",
  )
    .bind(claims.lic)
    .first<Licence>();

  if (!licence || licence.status !== "active") {
    return refuse("This licence is no longer active.", 403);
  }

  const seat = await env.DB.prepare("SELECT device FROM seats WHERE licence_id = ? AND device = ?")
    .bind(licence.id, device)
    .first();
  if (!seat) {
    return refuse("This machine is no longer on the licence. Enter your key again to add it.", 403);
  }

  await env.DB.prepare("UPDATE seats SET last_seen = ? WHERE licence_id = ? AND device = ?")
    .bind(now(), licence.id, device)
    .run();

  return grant(env, licence, device);
}

async function deactivate(env: Env, body: any) {
  const claims = await open(env, String(body.token ?? ""));
  // The client ignores this reply, so a failure here must not be loud. The
  // worst case is a seat that stays held until support frees it.
  if (!claims) return json({ ok: false });
  await env.DB.prepare("DELETE FROM seats WHERE licence_id = ? AND device = ?")
    .bind(claims.lic, String(body.device ?? claims.dev))
    .run();
  return json({ ok: true });
}

/** Creates a licence and returns the key. In production this is reached only
 *  from the processor's webhook; emailing the key is the operator's step. */
async function issue(
  env: Env,
  opts: { plan?: string; email?: string; orderRef?: string; renewsAt?: string; seats?: number },
) {
  const key = newLicenceKey();
  const id = `lc_${toHex(crypto.getRandomValues(new Uint8Array(8)))}`;
  await env.DB.prepare(
    `INSERT INTO licences (id, key_hash, plan, daily_limit, status, seat_limit, renews_at, email, order_ref, created_at)
     VALUES (?, ?, ?, NULL, 'active', ?, ?, ?, ?, ?)`,
  )
    .bind(
      id,
      await sha256Hex(key),
      opts.plan ?? "pro",
      opts.seats ?? DEFAULT_SEATS,
      opts.renewsAt ?? null,
      opts.email ?? null,
      opts.orderRef ?? null,
      now(),
    )
    .run();
  return { id, key };
}

/** Lemon Squeezy signs the raw body with HMAC-SHA256 and sends it as hex. */
async function webhookIsGenuine(env: Env, raw: string, signature: string): Promise<boolean> {
  if (!env.LEMONSQUEEZY_WEBHOOK_SECRET || !signature) return false;
  const mac = await crypto.subtle.importKey(
    "raw",
    enc.encode(env.LEMONSQUEEZY_WEBHOOK_SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"],
  );
  return crypto.subtle.verify("HMAC", mac, fromHex(signature), enc.encode(raw));
}

async function lemonSqueezy(env: Env, request: Request) {
  const raw = await request.text();
  if (!(await webhookIsGenuine(env, raw, request.headers.get("X-Signature") ?? ""))) {
    return refuse("Bad signature.", 401);
  }
  const event = JSON.parse(raw);
  const name = event?.meta?.event_name as string;
  const attrs = event?.data?.attributes ?? {};
  const orderRef = String(event?.data?.id ?? "");

  switch (name) {
    case "order_created":
    case "subscription_created": {
      const { key } = await issue(env, {
        email: attrs.user_email ?? attrs.customer_email,
        orderRef,
        renewsAt: attrs.renews_at ? String(attrs.renews_at).slice(0, 10) : undefined,
      });
      // The customer still has to receive this. Wire it to your mailer here;
      // until then it is in the licences table against their order.
      console.log(`issued licence for order ${orderRef}: ${key}`);
      return json({ ok: true });
    }
    case "subscription_cancelled":
    case "subscription_expired":
    case "order_refunded": {
      // No revocation list: flipping the status stops the next refresh and
      // the entitlement lapses on its own within the token's TTL.
      await env.DB.prepare("UPDATE licences SET status = ? WHERE order_ref = ?")
        .bind(name === "order_refunded" ? "refunded" : "cancelled", orderRef)
        .run();
      return json({ ok: true });
    }
    default:
      return json({ ok: true, ignored: name });
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const { pathname } = new URL(request.url);
    const dev = Boolean(env.DEV_MODE);

    try {
      if (request.method === "GET" && pathname === "/v1/pubkey" && dev) {
        return json({ public_key_hex: (await keys(env)).publicHex });
      }
      if (request.method !== "POST") return refuse("Not found.", 404);

      if (pathname === "/webhooks/lemonsqueezy") return lemonSqueezy(env, request);

      const body = await request.json().catch(() => ({}));
      switch (pathname) {
        case "/v1/activate":
          return await activate(env, body);
        case "/v1/refresh":
          return await refresh(env, body);
        case "/v1/deactivate":
          return await deactivate(env, body);
        case "/v1/dev/issue":
          return dev ? json(await issue(env, body as any)) : refuse("Not found.", 404);
        default:
          return refuse("Not found.", 404);
      }
    } catch (err) {
      // Never leak internals into a string the app will show to a user.
      console.error(err);
      return refuse("The licence service had a problem. Please try again shortly.", 500);
    }
  },
} satisfies ExportedHandler<Env>;
