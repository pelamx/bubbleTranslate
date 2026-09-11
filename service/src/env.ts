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

  /** Unlocks /admin. Unset means the route does not exist at all -- it answers
   *  404 rather than 401, so an unconfigured deployment does not advertise an
   *  admin panel to anyone scanning for one. */
  ADMIN_PASSWORD?: string;
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

/** The display prices. These are the product's prices, and they are also what
 *  the app itself says — see `PRICE_MONTHLY` in `src/license.rs`, which has to
 *  agree with these. Paddle prices the transaction in the buyer's own
 *  currency; these dollar figures are the fallback the buy page shows before
 *  Paddle's preview returns. */
export const USD_PRICE: Record<Cycle, string> = {
  monthly: "$2",
  yearly: "$20",
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
