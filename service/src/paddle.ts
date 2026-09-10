// Paddle, the single payment processor.
//
// Paddle is a merchant of record: it sells the licence to the customer and
// this service sells it to Paddle. That is worth the higher fee for a product
// sold to the world, because it makes VAT, sales tax and invoicing Paddle's
// problem rather than ours in every country at once.
//
// Checkout is Paddle.js in the browser, so no card and no address ever reaches
// this Worker; all that arrives here is a signed webhook saying it happened.

import { type Cycle, type Env, paddleApiBase } from "./env";
import { constantTimeEqual, hmacSha256, now, toHex } from "./tokens";

/** How far out of date a webhook's timestamp may be.
 *
 *  Paddle suggests five seconds. That is right for a server that is always
 *  warm and wrong for one that may be cold-starting an isolate, so this is
 *  deliberately looser — still short enough that a captured webhook cannot be
 *  replayed tomorrow, which is what the timestamp is in the signature for. */
const MAX_SKEW = 300;

export interface Verified {
  eventType: string;
  /** When Paddle says the event happened, RFC 3339. Not when it arrived:
   *  deliveries are unordered, so this is the only thing that can say which
   *  of two payloads for the same entity is the later one. */
  occurredAt: string | null;
  data: any;
}

/** Whether a delivery came from one of Paddle's own addresses.
 *
 *  Defence in depth, not the gate: the HMAC below is what actually proves a
 *  webhook is Paddle's, and it holds even if this returns `true` for someone
 *  else. What this buys is that a stranger who somehow learns the signing
 *  secret still has to arrive from Paddle's network.
 *
 *  The list is fetched rather than pasted in, because Paddle publishes it and
 *  can change it -- a hardcoded copy is a webhook outage waiting for the day
 *  they add an address. It is cached per isolate; a Worker isolate is
 *  short-lived, so this is a handful of requests a day, not one per webhook.
 *
 *  Fails **open**: if the list cannot be fetched, the delivery is allowed
 *  through to the signature check. Closing instead would mean a minute of
 *  trouble at Paddle's end costing us real payments, to protect a door the
 *  MAC already locks. The refusal is logged loudly either way. */
const IP_TTL = 3_600_000;
let ipCache: { at: number; cidrs: string[] } | null = null;

async function paddleIps(): Promise<string[] | null> {
  if (ipCache && Date.now() - ipCache.at < IP_TTL) return ipCache.cidrs;
  try {
    const res = await fetch("https://api.paddle.com/ips");
    if (!res.ok) throw new Error(`ips returned ${res.status}`);
    const body: any = await res.json();
    const cidrs: string[] = body?.data?.ipv4_cidrs ?? [];
    if (!cidrs.length) throw new Error("ips returned no ipv4_cidrs");
    ipCache = { at: Date.now(), cidrs };
    return cidrs;
  } catch (err) {
    console.error(`paddle ip list unavailable, allowing through to the MAC: ${err}`);
    return null;
  }
}

/** `a.b.c.d` as a 32-bit number, or null if it is not an IPv4 address. */
function ipv4(text: string): number | null {
  const parts = text.trim().split(".");
  if (parts.length !== 4) return null;
  let value = 0;
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const octet = Number(part);
    if (octet > 255) return null;
    value = (value << 8) | octet;
  }
  return value >>> 0;
}

function inCidr(address: number, cidr: string): boolean {
  const [base, lenText] = cidr.split("/");
  const start = ipv4(base ?? "");
  if (start === null) return false;
  const len = lenText === undefined ? 32 : Number(lenText);
  if (!Number.isInteger(len) || len < 0 || len > 32) return false;
  if (len === 0) return true;
  const mask = (0xffffffff << (32 - len)) >>> 0;
  return (address & mask) === (start & mask);
}

/** True when the request may proceed to signature checking. */
export async function fromPaddle(request: Request): Promise<boolean> {
  const cidrs = await paddleIps();
  if (!cidrs) return true;
  const seen = request.headers.get("CF-Connecting-IP") ?? "";
  const address = ipv4(seen);
  if (address === null) {
    console.error(`webhook from an address that is not IPv4: ${JSON.stringify(seen)}`);
    return false;
  }
  if (cidrs.some((cidr) => inCidr(address, cidr))) return true;
  console.error(`webhook refused: ${seen} is not one of Paddle's published addresses`);
  return false;
}

/** Checks the `Paddle-Signature` header against the raw body.
 *
 *  The signed value is `${ts}:${body}` — the timestamp is inside the MAC, so
 *  it cannot be edited to make an old webhook look fresh. Both halves have to
 *  hold: a valid signature over a stale timestamp is a replay. */
export async function verifyWebhook(
  env: Env,
  raw: string,
  header: string | null,
): Promise<Verified | null> {
  if (!env.PADDLE_WEBHOOK_SECRET || !header) return null;

  let ts = "";
  let h1 = "";
  for (const part of header.split(";")) {
    const [name, value] = part.split("=");
    if (name === "ts") ts = value ?? "";
    if (name === "h1") h1 = value ?? "";
  }
  if (!ts || !h1) return null;

  if (Math.abs(now() - Number(ts)) > MAX_SKEW) {
    console.error("Paddle webhook rejected: timestamp outside the allowed window");
    return null;
  }

  const expected = toHex(await hmacSha256(env.PADDLE_WEBHOOK_SECRET, `${ts}:${raw}`));
  if (!constantTimeEqual(expected, h1)) {
    console.error("Paddle webhook rejected: signature did not match");
    return null;
  }

  try {
    const event = JSON.parse(raw);
    return {
      eventType: String(event?.event_type ?? ""),
      occurredAt: event?.occurred_at ? String(event.occurred_at) : null,
      data: event?.data ?? {},
    };
  } catch {
    return null;
  }
}

/** Which of the two plans a transaction was for.
 *
 *  Read from the price id rather than from the amount: the amount varies with
 *  currency, tax and discounts, and none of those should be able to turn a
 *  monthly purchase into a yearly term. */
export function cycleFromItems(env: Env, items: any[]): Cycle | null {
  const ids = new Set(
    (items ?? []).map((item) => item?.price?.id ?? item?.price_id).filter(Boolean),
  );
  if (env.PADDLE_PRICE_YEARLY && ids.has(env.PADDLE_PRICE_YEARLY)) return "yearly";
  if (env.PADDLE_PRICE_MONTHLY && ids.has(env.PADDLE_PRICE_MONTHLY)) return "monthly";
  return null;
}

/** Best effort at the buyer's address, for the licence email.
 *
 *  Paddle does not put the email in every payload shape, so this tries the
 *  places it does appear and then asks the API. A licence with no address on
 *  it is still a working licence — the key was shown on the success page — so
 *  a failure here is logged, not raised. */
export async function customerEmail(env: Env, data: any): Promise<string | null> {
  const inline =
    data?.customer?.email ??
    data?.billing_details?.email ??
    data?.custom_data?.email ??
    null;
  if (inline) return String(inline);

  const customerId = data?.customer_id;
  if (!customerId || !env.PADDLE_API_KEY) return null;

  try {
    const response = await fetch(`${paddleApiBase(env)}/customers/${customerId}`, {
      headers: { authorization: `Bearer ${env.PADDLE_API_KEY}` },
    });
    if (!response.ok) return null;
    const body: any = await response.json();
    return body?.data?.email ?? null;
  } catch (err) {
    console.error("could not look up the Paddle customer", err);
    return null;
  }
}

/** Cancels at the end of the paid period rather than immediately: the
 *  subscriber has paid for the rest of the term and is entitled to it, which
 *  is also why `endLicence("cancelled")` leaves `expires_at` alone. */
/** What went wrong, as a key into the page strings — the page says it in the
 *  reader's language; the log line beside it says the detail in English. */
export type CancelFailure =
  | "cancelNotConfigured"
  | "paddleRefusedCancel"
  | "paddleRefusedPortal"
  | "paddleUnreachable";

export async function cancelSubscription(
  env: Env,
  subscriptionId: string,
): Promise<CancelFailure | null> {
  if (!env.PADDLE_API_KEY) return "cancelNotConfigured";
  try {
    const response = await fetch(
      `${paddleApiBase(env)}/subscriptions/${subscriptionId}/cancel`,
      {
        method: "POST",
        headers: {
          authorization: `Bearer ${env.PADDLE_API_KEY}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({ effective_from: "next_billing_period" }),
      },
    );
    if (!response.ok) {
      console.error(`Paddle refused the cancellation (${response.status}): ${await response.text()}`);
      return "paddleRefusedCancel";
    }
    return null;
  } catch (err) {
    console.error("could not reach Paddle to cancel", err);
    return "paddleUnreachable";
  }
}

/** Mints a customer portal session and returns the URL to send the browser to.
 *
 *  The portal is Paddle-hosted, and that is the point: updating a card,
 *  downloading an invoice and cancelling all happen on Paddle's side, so no
 *  card number and no billing address ever reaches this Worker -- the same
 *  reason checkout is Paddle.js rather than a form here.
 *
 *  The customer id is never taken from the browser. Callers resolve it from
 *  the licence key the visitor proved they hold; a customer id in a form field
 *  is an invitation to read someone else's invoices.
 *
 *  Sessions are short-lived by design, so the URL is redirected to immediately
 *  and never stored. */
export async function portalSession(
  env: Env,
  customerId: string,
  subscriptionIds: string[] = [],
): Promise<string | CancelFailure> {
  if (!env.PADDLE_API_KEY) return "cancelNotConfigured";
  try {
    const response = await fetch(
      `${paddleApiBase(env)}/customers/${customerId}/portal-sessions`,
      {
        method: "POST",
        headers: {
          authorization: `Bearer ${env.PADDLE_API_KEY}`,
          "content-type": "application/json",
        },
        body: JSON.stringify(
          subscriptionIds.length ? { subscription_ids: subscriptionIds } : {},
        ),
      },
    );
    if (!response.ok) {
      console.error(`Paddle refused a portal session (${response.status}): ${await response.text()}`);
      return "paddleRefusedPortal";
    }
    const body: any = await response.json();
    // The overview, not one of the deep links: this is a general "manage
    // billing" button, and landing someone who came to read an invoice on the
    // change-your-card screen is worse than one more click. The deep links are
    // only a fallback for a response that somehow has no overview.
    const urls = body?.data?.urls ?? {};
    const url =
      urls?.general?.overview ??
      (urls.subscriptions ?? [])[0]?.update_subscription_payment_method;
    if (!url) {
      console.error("Paddle returned a portal session with no URL");
      return "paddleRefusedPortal";
    }
    return String(url);
  } catch (err) {
    console.error("could not reach Paddle to open the portal", err);
    return "paddleUnreachable";
  }
}
