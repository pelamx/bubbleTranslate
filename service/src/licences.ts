// Licences: creating one, finding it again when the processor calls back, and
// turning it into the token the app checks on its own.

import { type Cycle, type Env, TERM_SECONDS, supportEmail } from "./env";
import { mint, now, randomHex, sha256Hex } from "./tokens";

/** Matches the client's TOKEN_TTL_HINT. Long enough that a service outage is
 *  invisible to a paying user, short enough that a lapse is not permanent. */
export const TOKEN_TTL = 30 * 86_400;

/** How long an entitlement outlives the paid term it was bought with.
 *
 *  This is the number that makes cancelling honest. Without it a token is
 *  always minted for thirty days, so someone who cancels a $2 monthly licence
 *  the day after paying keeps Pro for a further month — a third of a year's
 *  revenue given away by an off-by-one in the TTL. With it, the token can
 *  never outlive the term by more than a few days' grace for a late renewal. */
export const GRACE_SECONDS = 3 * 86_400;

export const DEFAULT_SEATS = 3;

/** The `provider` on a licence that was issued by hand rather than bought.
 *
 *  Kept distinct from `paytr` and `paddle` for two reasons. A row nobody was
 *  charged for must not be counted as revenue alongside the ones that were;
 *  and `licenceByProviderRef` looks a licence up by (provider, ref), so a
 *  hand-issued licence can never be found — and extended, or cancelled — by a
 *  webhook that happens to carry a matching reference. */
export const MANUAL_PROVIDER = "manual";

export interface Licence {
  id: string;
  plan: string;
  cycle: string;
  translation_limit: number | null;
  status: string;
  seat_limit: number;
  expires_at: number;
  renews_at: string | null;
  email: string | null;
  provider: string;
  provider_ref: string | null;
}

const COLUMNS =
  "id, plan, cycle, translation_limit, status, seat_limit, expires_at, renews_at, email, provider, provider_ref";

// -- keys --------------------------------------------------------------------

/** No I, L, O, 0 or 1: this gets read off a page or an email and typed into a
 *  text box, sometimes from a phone screen. */
const ALPHABET = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";

export function newLicenceKey(): string {
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

const asDate = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

// -- the token the app gets --------------------------------------------------

export async function grantToken(env: Env, licence: Licence, device: string) {
  const issued = now();
  // The term is the ceiling, not the TTL. Whichever runs out first decides.
  const exp = Math.min(issued + TOKEN_TTL, licence.expires_at + GRACE_SECONDS);
  const token = await mint(env, {
    lic: licence.id,
    plan: licence.plan,
    ...(licence.cycle ? { cyc: licence.cycle } : {}),
    lim: licence.translation_limit,
    dev: device,
    iat: issued,
    exp,
  });
  return { token, renews: licence.renews_at };
}

/** Whether this licence still entitles anyone to anything.
 *
 *  The term is what decides, not the status. That is deliberate and it is the
 *  whole reason `endLicence("cancelled")` leaves `expires_at` alone: someone
 *  who cancels a yearly licence in month two has paid for ten more months and
 *  is entitled to them. Reading `cancelled` as "off now" would take back time
 *  they bought — the same mistake as billing them for it, pointed the other
 *  way, and the one people write to their bank about.
 *
 *  A refund is the exception, because the money went back. `endLicence` also
 *  cuts its term to now, so the second condition would catch it anyway; the
 *  status is checked first so that a refund whose term-cut failed to write
 *  still refuses. */
export function isLive(licence: Licence): boolean {
  if (licence.status === "refunded") return false;
  return licence.expires_at + GRACE_SECONDS > now();
}

/** Why a licence is refused, phrased for the person holding it — the client
 *  shows these strings verbatim. */
export function refusalFor(licence: Licence, env: Env): string {
  if (licence.status === "refunded") {
    return "This licence was refunded and is no longer active.";
  }
  if (licence.expires_at + GRACE_SECONDS <= now()) {
    return licence.status === "cancelled"
      ? "This subscription was cancelled and its paid period has ended."
      : "This licence has expired. Renew it to carry on using Pro.";
  }
  // Everything reachable is covered above: `isLive` only refuses a refund or
  // an ended term, and a cancelled licence inside its paid period is not
  // refused at all. This is the line that runs if that ever stops being true.
  return `This licence is not active. Contact ${supportEmail(env)} if that is unexpected.`;
}

// -- lookups -----------------------------------------------------------------

export const licenceById = (env: Env, id: string) =>
  env.DB.prepare(`SELECT ${COLUMNS} FROM licences WHERE id = ?`).bind(id).first<Licence>();

export const licenceByKey = async (env: Env, key: string) =>
  env.DB.prepare(`SELECT ${COLUMNS} FROM licences WHERE key_hash = ?`)
    .bind(await sha256Hex(key.trim().toUpperCase()))
    .first<Licence>();

export const licenceByProviderRef = (env: Env, provider: string, ref: string) =>
  env.DB.prepare(`SELECT ${COLUMNS} FROM licences WHERE provider = ? AND provider_ref = ?`)
    .bind(provider, ref)
    .first<Licence>();

// -- creating and moving one -------------------------------------------------

export interface IssueOptions {
  provider: string;
  cycle: Cycle;
  email?: string | null;
  providerRef?: string | null;
  seats?: number;
  /** Overrides the term length. Only the dev route uses it. */
  termSeconds?: number;
}

export async function issueLicence(env: Env, opts: IssueOptions) {
  const key = newLicenceKey();
  const id = `lc_${randomHex(8)}`;
  const expiresAt = now() + (opts.termSeconds ?? TERM_SECONDS[opts.cycle]);

  await env.DB.prepare(
    `INSERT INTO licences
       (id, key_hash, plan, cycle, translation_limit, status, seat_limit,
        expires_at, renews_at, email, provider, provider_ref, created_at)
     VALUES (?, ?, 'pro', ?, NULL, 'active', ?, ?, ?, ?, ?, ?, ?)`,
  )
    .bind(
      id,
      await sha256Hex(key),
      opts.cycle,
      opts.seats ?? DEFAULT_SEATS,
      expiresAt,
      asDate(expiresAt),
      opts.email ?? null,
      opts.provider,
      opts.providerRef ?? null,
      now(),
    )
    .run();

  return { id, key, expiresAt };
}

/** Moves the paid term forward by one cycle. Called on a renewal.
 *
 *  Extends from whichever is later, the current expiry or now: renewing early
 *  must add to the term rather than truncate it, and renewing after a lapse
 *  must not credit the time the licence spent expired. */
export async function extendTerm(env: Env, licence: Licence, cycle: Cycle) {
  const from = Math.max(licence.expires_at, now());
  const expiresAt = from + TERM_SECONDS[cycle];
  await env.DB.prepare(
    "UPDATE licences SET expires_at = ?, renews_at = ?, cycle = ?, status = 'active' WHERE id = ?",
  )
    .bind(expiresAt, asDate(expiresAt), cycle, licence.id)
    .run();
  return expiresAt;
}

/** Issues a fresh key for an existing licence, and returns it.
 *
 *  This is the answer to "I lost my key". It cannot be looked up: `licences`
 *  stores only the hash, and the plaintext copy in `orders` is swept within
 *  the hour. That is deliberate — a stolen database should not be a stolen
 *  list of working licences — and the cost of it is that recovery means
 *  replacement.
 *
 *  Replacement, not a second licence: the same row keeps its id, its term, its
 *  subscription and its seats. Machines already activated are undisturbed,
 *  because `refresh` finds the licence by the `lic` claim in the token rather
 *  than by the key. Only the old key stops working, which is also what makes
 *  this the right tool for a key that leaked rather than one that was lost. */
export async function rotateKey(env: Env, licence: Licence): Promise<string> {
  const key = newLicenceKey();
  await env.DB.prepare("UPDATE licences SET key_hash = ? WHERE id = ?")
    .bind(await sha256Hex(key), licence.id)
    .run();
  return key;
}

/** Ends a licence. `refunded` cuts the term immediately, because the money has
 *  gone back; `cancelled` leaves the term alone, because it has been paid for
 *  and the subscriber is entitled to the rest of it. */
export async function endLicence(env: Env, licence: Licence, status: "cancelled" | "refunded") {
  if (status === "refunded") {
    await env.DB.prepare("UPDATE licences SET status = ?, expires_at = ? WHERE id = ?")
      .bind(status, now(), licence.id)
      .run();
  } else {
    await env.DB.prepare("UPDATE licences SET status = ? WHERE id = ?")
      .bind(status, licence.id)
      .run();
  }
}

// -- orders ------------------------------------------------------------------

export interface Order {
  ref: string;
  provider: string;
  cycle: string;
  email: string | null;
  status: string;
  amount: number | null;
  currency: string | null;
  licence_id: string | null;
  licence_key: string | null;
  reveal_until: number | null;
  failure: string | null;
}

/** How long the success page will still say the key out loud. Long enough to
 *  survive a slow redirect, a closed tab and a reopened one; short enough that
 *  the orders table is not a standing list of live licences. */
export const REVEAL_SECONDS = 3600;

/** The ref is this order's whole access control — the success page reveals the
 *  key to whoever holds it — so it is unguessable: bare hex, 128 bits of it. */
export const newOrderRef = () => randomHex(16);

export async function createOrder(
  env: Env,
  order: { ref: string; provider: string; cycle: Cycle; email?: string | null; amount: number; currency: string },
) {
  await env.DB.prepare(
    `INSERT INTO orders (ref, provider, cycle, email, status, amount, currency, created_at)
     VALUES (?, ?, ?, ?, 'pending', ?, ?, ?)`,
  )
    .bind(order.ref, order.provider, order.cycle, order.email ?? null, order.amount, order.currency, now())
    .run();
}

export const orderByRef = (env: Env, ref: string) =>
  env.DB.prepare(
    `SELECT ref, provider, cycle, email, status, amount, currency, licence_id, licence_key,
            reveal_until, failure
       FROM orders WHERE ref = ?`,
  )
    .bind(ref)
    .first<Order>();

export async function markOrderPaid(env: Env, ref: string, licenceId: string, key: string) {
  await env.DB.prepare(
    "UPDATE orders SET status = 'paid', licence_id = ?, licence_key = ?, reveal_until = ? WHERE ref = ?",
  )
    .bind(licenceId, key, now() + REVEAL_SECONDS, ref)
    .run();
}

export async function markOrderFailed(env: Env, ref: string, reason: string) {
  await env.DB.prepare("UPDATE orders SET status = 'failed', failure = ? WHERE ref = ?")
    .bind(reason.slice(0, 200), ref)
    .run();
}

/** Clears keys whose reveal window has passed.
 *
 *  Called opportunistically from the reveal route rather than from a schedule:
 *  a cron trigger would be tidier, but this service must keep working if one
 *  is never configured, and the invariant that matters — no plaintext key
 *  outlives its hour by long — holds either way. */
export async function sweepRevealedKeys(env: Env) {
  await env.DB.prepare(
    "UPDATE orders SET licence_key = NULL WHERE licence_key IS NOT NULL AND reveal_until < ?",
  )
    .bind(now())
    .run();
}

// -- delivery ----------------------------------------------------------------

/** Mails the key, if a mailer is configured.
 *
 *  Deliberately best-effort and deliberately not the primary route: the
 *  success page has already shown the key by the time this runs. A webhook
 *  that failed because a mail API was down would be retried by the processor
 *  and could issue a second licence for one payment, which is a worse failure
 *  than an email that never arrives.
 */
export async function deliverKey(
  env: Env,
  to: string | null | undefined,
  key: string,
  cycle: string,
  expiresAt: number,
): Promise<void> {
  if (!to) return;
  if (!env.RESEND_API_KEY || !env.MAIL_FROM) {
    console.log(`no mailer configured; licence ${key} for ${to} was shown on the success page only`);
    return;
  }
  try {
    const response = await fetch("https://api.resend.com/emails", {
      method: "POST",
      headers: {
        authorization: `Bearer ${env.RESEND_API_KEY}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({
        from: env.MAIL_FROM,
        to,
        subject: "Your bubbleTranslate Pro licence key",
        text: [
          "Thanks for subscribing to bubbleTranslate Pro.",
          "",
          `  ${key}`,
          "",
          "Open bubbleTranslate, go to the Account section, paste the key and",
          "choose Activate. It works on up to three machines.",
          "",
          `Plan: Pro (${cycle}). Next renewal: ${asDate(expiresAt)}.`,
          "",
          `Questions: ${supportEmail(env)}`,
        ].join("\n"),
      }),
    });
    if (!response.ok) {
      console.error(`mailer refused the licence email (${response.status})`);
    }
  } catch (err) {
    console.error("could not send the licence email", err);
  }
}
