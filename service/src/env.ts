// What the service is configured with, and the two plans it sells.

export interface Env {
  DB: D1Database;

  /** Opens the dev-only routes and lets the service keep its own key. */
  DEV_MODE?: string;
  /** Where this service is reachable, without a trailing slash. Used to build
   *  the success URL Paddle sends the buyer back to, and the webhook
   *  destination — neither of which can be inferred from an inbound request. */
  PUBLIC_BASE_URL?: string;

  /** Production signing key: base64url PKCS#8, set with `wrangler secret put`. */
  SIGNING_KEY_PKCS8?: string;
  /** Its public half, 32 bytes as hex. This is what goes in PUBLIC_KEY_HEX. */
  SIGNING_KEY_PUBLIC?: string;

  // -- Paddle ----------------------------------------------------------------
  /** Sandbox until it is set to "production". Chooses both the API host and
   *  the Paddle.js environment, so the two can never disagree. */
  PADDLE_ENV?: string;
  PADDLE_API_KEY?: string;
  PADDLE_WEBHOOK_SECRET?: string;
  /** The browser-side token Paddle.js is initialised with. Publishable by
   *  design — it is in the page source — and not a secret. */
  PADDLE_CLIENT_TOKEN?: string;
  /** Price ids from the Paddle catalogue, `pri_...`. */
  PADDLE_PRICE_MONTHLY?: string;
  PADDLE_PRICE_YEARLY?: string;

  // -- Delivering the key ----------------------------------------------------
  /** Optional. Without it the key is shown on the success page and logged,
   *  which is enough to buy and use the app but leaves the customer nothing
   *  to search their mail for later. */
  RESEND_API_KEY?: string;
  MAIL_FROM?: string;
  SUPPORT_EMAIL?: string;
  /** Where the weekly report is emailed. Falls back to SUPPORT_EMAIL. */
  REPORT_EMAIL?: string;

  // -- Ad conversion attribution --------------------------------------------
  //
  // All optional. With none set, a sale pushes no conversion (see ads.ts): the
  // feature is dark until its secrets are put, exactly like mail delivery.
  // Everything but the numeric ids is a secret -- set with `wrangler secret put`.

  /** Google Ads offline click-conversion upload. The customer id is digits
   *  only, no dashes; login customer id is the MCC when the account is under
   *  one; the conversion action is the full resource name
   *  `customers/<id>/conversionActions/<id>`. */
  GOOGLE_ADS_DEVELOPER_TOKEN?: string;
  GOOGLE_ADS_CLIENT_ID?: string;
  GOOGLE_ADS_CLIENT_SECRET?: string;
  GOOGLE_ADS_REFRESH_TOKEN?: string;
  GOOGLE_ADS_CUSTOMER_ID?: string;
  GOOGLE_ADS_LOGIN_CUSTOMER_ID?: string;
  GOOGLE_ADS_CONVERSION_ACTION?: string;
  /** The Google tag id, `AW-...`, loaded on the success page. Publishable:
   *  it is in the page source of the marketing site too. */
  GOOGLE_ADS_TAG_ID?: string;
  /** The purchase conversion's label, the part after the `/` in the event
   *  snippet's `send_to`. Without it the success page loads no Google tag. */
  GOOGLE_ADS_PURCHASE_LABEL?: string;

  /** Meta (Facebook) Conversions API. The pixel id is not secret; the access
   *  token is. A test event code, when set, routes events to the Test Events
   *  tab instead of reporting. */
  META_PIXEL_ID?: string;
  META_ACCESS_TOKEN?: string;
  META_TEST_EVENT_CODE?: string;

  /** Unlocks /admin. Unset means the route does not exist at all -- it answers
   *  404 rather than 401, so an unconfigured deployment does not advertise an
   *  admin panel to anyone scanning for one. */
  ADMIN_PASSWORD?: string;
  /** The operator's own install ids and licence emails, comma-separated, left
   *  out of the admin panel's counts so they show real users only. Secrets
   *  rather than vars because the emails are personal. */
  ADMIN_IGNORE_INSTALLS?: string;
  ADMIN_IGNORE_EMAILS?: string;
}

export type Cycle = "monthly" | "yearly";

export const CYCLES: Cycle[] = ["monthly", "yearly"];

export const isCycle = (value: unknown): value is Cycle =>
  value === "monthly" || value === "yearly";

/** How long a paid term lasts. A month is 30 days rather than a calendar
 *  month: this number decides when an entitlement lapses, and a subscriber
 *  whose renewal is a few hours late should not spend them locked out. The
 *  processor's own renewal date is what the customer is shown. */
export const TERM_SECONDS: Record<Cycle, number> = {
  monthly: 31 * 86_400,
  yearly: 366 * 86_400,
};

/** The product's price in whole US dollars, and the one place it is written.
 *  Everything that needs a price derives from this: the display strings below,
 *  the value reported to the ad networks (`ads.ts`) and the revenue in the
 *  weekly report (`report.ts`). Those last two are money figures that nobody
 *  eyeballs week to week, so a second hardcoded copy would drift silently. */
export const USD_AMOUNT: Record<Cycle, number> = {
  monthly: 2,
  yearly: 20,
};

/** The display prices. These are the product's prices, and they are also what
 *  the app itself says — see `PRICE_MONTHLY` in `src/license.rs`, which has to
 *  agree with these. Paddle prices the transaction in the buyer's own
 *  currency; these dollar figures are the fallback the buy page shows before
 *  Paddle's preview returns. */
export const USD_PRICE: Record<Cycle, string> = {
  monthly: `$${USD_AMOUNT.monthly}`,
  yearly: `$${USD_AMOUNT.yearly}`,
};

export function paddlePriceId(env: Env, cycle: Cycle): string | null {
  const id = cycle === "monthly" ? env.PADDLE_PRICE_MONTHLY : env.PADDLE_PRICE_YEARLY;
  return id && id.trim() ? id.trim() : null;
}

export const paddleConfigured = (env: Env) =>
  Boolean(env.PADDLE_CLIENT_TOKEN && (paddlePriceId(env, "monthly") || paddlePriceId(env, "yearly")));

/** The one place `PADDLE_ENV` is read, and it refuses to guess.
 *
 *  An unset or misspelled value used to mean "sandbox", which is the wrong
 *  default in the only direction that matters: it points a deployment that
 *  believes it is live at the sandbox account, where the price ids do not
 *  exist and the webhook secret does not match. Failing here instead turns a
 *  silent wrong-account run into a loud one. */
export function paddleEnvOrThrow(env: Env): "production" | "sandbox" {
  const raw = (env.PADDLE_ENV ?? "").trim();
  if (raw === "production" || raw === "sandbox") return raw;
  throw new Error(
    `PADDLE_ENV must be "sandbox" or "production", got ${JSON.stringify(raw)}. ` +
      `Set it in wrangler.toml [vars]; it chooses both the API host and the ` +
      `Paddle.js environment, so it is never inferred.`,
  );
}

export const paddleApiBase = (env: Env) =>
  paddleEnvOrThrow(env) === "production"
    ? "https://api.paddle.com"
    : "https://sandbox-api.paddle.com";

export const supportEmail = (env: Env) => env.SUPPORT_EMAIL ?? "support@bubbletranslate.app";

export function baseUrl(env: Env, request: Request): string {
  if (env.PUBLIC_BASE_URL) return env.PUBLIC_BASE_URL.replace(/\/+$/, "");
  return new URL(request.url).origin;
}


// -- Ad conversion configuration ---------------------------------------------
//
// Each platform is independent: Google may be wired while Meta is not. A push
// is only attempted for a platform that is fully configured, so a half-set
// account never throws on the webhook path.

export const googleAdsConfigured = (env: Env) =>
  Boolean(
    env.GOOGLE_ADS_DEVELOPER_TOKEN &&
      env.GOOGLE_ADS_CLIENT_ID &&
      env.GOOGLE_ADS_CLIENT_SECRET &&
      env.GOOGLE_ADS_REFRESH_TOKEN &&
      env.GOOGLE_ADS_CUSTOMER_ID &&
      env.GOOGLE_ADS_CONVERSION_ACTION,
  );

export const metaAdsConfigured = (env: Env) =>
  Boolean(env.META_PIXEL_ID && env.META_ACCESS_TOKEN);
