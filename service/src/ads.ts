// Pushing a paid conversion back to the ad networks.
//
// This is off the translation path and off the buying path: it runs after a
// licence has already been issued, and it is best-effort by design. A network
// that is slow, down, or misconfigured must never turn a paid order into a
// retried webhook -- Paddle retries on any non-2xx, and a retry here would
// re-issue nothing (fulfilment is idempotent) but would double-count the
// conversion. So every call swallows its own failures and only logs.
//
// No SDK: the service is a Worker. Google and Meta are both plain HTTPS, and
// the only crypto needed is SHA-256 for Meta's hashed email, which WebCrypto
// gives us.

import {
  Env,
  Cycle,
  USD_AMOUNT,
  googleAdsConfigured,
  metaAdsConfigured,
} from "./env";

/** The click ids that travelled from the landing page through Paddle's
 *  custom_data. Any subset may be present; a platform is skipped when it has
 *  nothing it can match on. */
export interface ClickIds {
  gclid?: string;
  gbraid?: string;
  wbraid?: string;
  fbc?: string;
  fbp?: string;
}

export interface ConversionInput {
  clickIds: ClickIds;
  cycle: Cycle;
  /** The order ref, reused as the dedupe key on both networks. */
  orderRef: string;
  /** For Meta's hashed-email match. Optional; fbc/fbp alone are enough. */
  email?: string | null;
  /** When the purchase happened, ms since epoch. */
  eventTimeMs: number;
}

// The conversion is reported in USD at the product's list price, taken from
// USD_AMOUNT in env.ts. Paddle charges the buyer in their own currency; sending
// that back would mean dividing a minor-unit string by 100, which is wrong for
// zero-decimal currencies (JPY, KRW). A stable USD value keeps ROAS honest and
// sidesteps the whole currency-unit question.

/** Fan out to whichever networks are configured. Never throws. */
export async function pushConversions(env: Env, input: ConversionInput): Promise<void> {
  const value = USD_AMOUNT[input.cycle];
  await Promise.allSettled([
    maybeGoogle(env, input, value),
    maybeMeta(env, input, value),
  ]);
}

async function maybeGoogle(env: Env, input: ConversionInput, value: number): Promise<void> {
  const { gclid, gbraid, wbraid } = input.clickIds;
  if (!googleAdsConfigured(env)) return;
  if (!gclid && !gbraid && !wbraid) return; // nothing Google can match on
  try {
    await pushGoogleAdsConversion(env, input, value);
  } catch (err) {
    console.error("google ads conversion push failed", err);
  }
}

async function maybeMeta(env: Env, input: ConversionInput, value: number): Promise<void> {
  const { fbc, fbp } = input.clickIds;
  if (!metaAdsConfigured(env)) return;
  if (!fbc && !fbp && !input.email) return; // no identifier to match on
  try {
    await pushMetaConversion(env, input, value);
  } catch (err) {
    console.error("meta conversion push failed", err);
  }
}

// -- Google Ads --------------------------------------------------------------

/** Exchange the offline refresh token for a short-lived access token. */
async function googleAccessToken(env: Env): Promise<string> {
  const res = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: env.GOOGLE_ADS_CLIENT_ID!,
      client_secret: env.GOOGLE_ADS_CLIENT_SECRET!,
      refresh_token: env.GOOGLE_ADS_REFRESH_TOKEN!,
      grant_type: "refresh_token",
    }),
  });
  if (!res.ok) throw new Error(`token exchange ${res.status}: ${await res.text()}`);
  const body: any = await res.json();
  if (!body.access_token) throw new Error("token exchange returned no access_token");
  return String(body.access_token);
}

async function pushGoogleAdsConversion(
  env: Env,
  input: ConversionInput,
  value: number,
): Promise<void> {
  const token = await googleAccessToken(env);
  const customerId = env.GOOGLE_ADS_CUSTOMER_ID!.replace(/\D/g, "");

  const conversion: Record<string, unknown> = {
    conversionAction: env.GOOGLE_ADS_CONVERSION_ACTION,
    conversionDateTime: googleDateTime(input.eventTimeMs),
    conversionValue: value,
    currencyCode: "USD",
    // Reusing the order ref lets Google dedupe if this ever fires twice.
    orderId: input.orderRef,
  };
  if (input.clickIds.gclid) conversion.gclid = input.clickIds.gclid;
  else if (input.clickIds.gbraid) conversion.gbraid = input.clickIds.gbraid;
  else if (input.clickIds.wbraid) conversion.wbraid = input.clickIds.wbraid;

  const headers: Record<string, string> = {
    "content-type": "application/json",
    authorization: `Bearer ${token}`,
    "developer-token": env.GOOGLE_ADS_DEVELOPER_TOKEN!,
  };
  if (env.GOOGLE_ADS_LOGIN_CUSTOMER_ID) {
    headers["login-customer-id"] = env.GOOGLE_ADS_LOGIN_CUSTOMER_ID.replace(/\D/g, "");
  }

  const url = `https://googleads.googleapis.com/v18/customers/${customerId}:uploadClickConversions`;
  const res = await fetch(url, {
    method: "POST",
    headers,
    // partialFailure lets a single bad conversion be reported without failing
    // the request; we read it back and log rather than throw.
    body: JSON.stringify({ conversions: [conversion], partialFailure: true }),
  });
  if (!res.ok) throw new Error(`upload ${res.status}: ${await res.text()}`);
  const body: any = await res.json().catch(() => ({}));
  if (body.partialFailureError) {
    console.error("google ads partial failure", JSON.stringify(body.partialFailureError));
  } else {
    console.log(`google ads conversion pushed for order ${input.orderRef}`);
  }
}

/** Google wants "yyyy-mm-dd hh:mm:ss+00:00", in a real time zone. UTC it is. */
function googleDateTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ` +
    `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}+00:00`
  );
}

// -- Meta Conversions API ----------------------------------------------------

async function pushMetaConversion(
  env: Env,
  input: ConversionInput,
  value: number,
): Promise<void> {
  const userData: Record<string, unknown> = {};
  if (input.clickIds.fbc) userData.fbc = input.clickIds.fbc;
  if (input.clickIds.fbp) userData.fbp = input.clickIds.fbp;
  if (input.email) userData.em = [await sha256Hex(input.email.trim().toLowerCase())];

  const event: Record<string, unknown> = {
    event_name: "Purchase",
    event_time: Math.floor(input.eventTimeMs / 1000),
    action_source: "website",
    // Same key as Google's orderId: Meta dedupes events sharing an event_id.
    event_id: input.orderRef,
    user_data: userData,
    custom_data: { currency: "USD", value },
  };

  const payload: Record<string, unknown> = { data: [event] };
  if (env.META_TEST_EVENT_CODE) payload.test_event_code = env.META_TEST_EVENT_CODE;

  const url =
    `https://graph.facebook.com/v19.0/${env.META_PIXEL_ID}/events` +
    `?access_token=${encodeURIComponent(env.META_ACCESS_TOKEN!)}`;
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!res.ok) throw new Error(`meta events ${res.status}: ${await res.text()}`);
  console.log(`meta conversion pushed for order ${input.orderRef}`);
}

async function sha256Hex(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
