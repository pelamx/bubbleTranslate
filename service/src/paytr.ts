// PayTR, which is how Turkey pays.
//
// The iframe API: we ask PayTR for a token describing the charge, drop their
// iframe on a page of ours, and PayTR tells us out of band whether the card
// went through. The buyer's card details never touch this service, which is
// the whole reason to use a processor rather than take a card ourselves.
//
// One thing to know before reading further: these licences are **fixed-term,
// not auto-renewing**. PayTR's recurring product has to be enabled per
// merchant account and is not available to every seller, so the default here
// is a term the buyer chooses and re-buys — a year for the yearly price, a
// month for the monthly one. `extendTerm` already handles a renewal arriving
// for an existing licence, so turning recurring on later is a webhook change
// rather than a redesign. See the README.

import { type Cycle, type Env, paytrPriceKurus } from "./env";
import { b64, constantTimeEqual, hmacSha256 } from "./tokens";

const TOKEN_ENDPOINT = "https://www.paytr.com/odeme/api/get-token";
const IFRAME_BASE = "https://www.paytr.com/odeme/guvenli";

/** PayTR's name for the Turkish lira. Not "TRY" — their API takes "TL", and
 *  sending the ISO code silently prices the basket in the account default. */
const CURRENCY = "TL";

/** Minutes PayTR keeps the payment page open. Their default is 30; this is
 *  shorter because an abandoned page holds an order row open. */
const TIMEOUT_MINUTES = 30;

export const iframeUrl = (token: string) => `${IFRAME_BASE}/${token}`;

export interface ChargeRequest {
  ref: string;
  cycle: Cycle;
  email: string;
  userIp: string;
  okUrl: string;
  failUrl: string;
}

export interface ChargeResult {
  ok: boolean;
  token?: string;
  amount?: number;
  error?: string;
}

/** The basket PayTR shows the buyer, base64 of `[[name, price, count]]`.
 *  Price is a decimal string of lira here, unlike `payment_amount`, which is
 *  an integer of kuruş. Their API genuinely wants both, in those two units. */
function basket(cycle: Cycle, kurus: number): string {
  const name = cycle === "yearly" ? "bubbleTranslate Pro — 1 yıl" : "bubbleTranslate Pro — 1 ay";
  const lira = (kurus / 100).toFixed(2);
  return btoa(
    unescape(encodeURIComponent(JSON.stringify([[name, lira, 1]]))),
  );
}

/** Asks PayTR to open a payment session, and returns the token its iframe is
 *  addressed by. */
export async function createCharge(env: Env, req: ChargeRequest): Promise<ChargeResult> {
  const merchantId = env.PAYTR_MERCHANT_ID!;
  const merchantKey = env.PAYTR_MERCHANT_KEY!;
  const merchantSalt = env.PAYTR_MERCHANT_SALT!;

  const amount = paytrPriceKurus(env, req.cycle);
  if (amount === null) {
    return {
      ok: false,
      error:
        "The lira price for this plan has not been set. " +
        "Set PAYTR_PRICE_MONTHLY_KURUS and PAYTR_PRICE_YEARLY_KURUS.",
    };
  }

  const testMode = env.PAYTR_TEST_MODE === "1" ? "1" : "0";
  const noInstallment = "0";
  const maxInstallment = "0";
  const userBasket = basket(req.cycle, amount);

  // The order of these fields is the signature. PayTR concatenates them in
  // exactly this sequence, appends the salt, and HMACs the result with the
  // merchant key — so a field moved is a token refused, with no explanation
  // beyond "invalid hash".
  const hashStr =
    merchantId +
    req.userIp +
    req.ref +
    req.email +
    String(amount) +
    userBasket +
    noInstallment +
    maxInstallment +
    CURRENCY +
    testMode;
  const paytrToken = b64(await hmacSha256(merchantKey, hashStr + merchantSalt));

  const form = new URLSearchParams({
    merchant_id: merchantId,
    user_ip: req.userIp,
    merchant_oid: req.ref,
    email: req.email,
    payment_amount: String(amount),
    paytr_token: paytrToken,
    user_basket: userBasket,
    debug_on: env.DEV_MODE ? "1" : "0",
    no_installment: noInstallment,
    max_installment: maxInstallment,
    currency: CURRENCY,
    test_mode: testMode,
    // PayTR requires all three, and rejects empty strings. There is no
    // address to give: this is a licence key delivered to an inbox, so the
    // fields say so rather than inventing a shipping address.
    user_name: req.email.split("@")[0].slice(0, 60) || "bubbleTranslate",
    user_address: "Dijital teslimat — fiziksel adres yok",
    user_phone: "0000000000",
    merchant_ok_url: req.okUrl,
    merchant_fail_url: req.failUrl,
    timeout_limit: String(TIMEOUT_MINUTES),
    lang: "tr",
  });

  let payload: any;
  try {
    const response = await fetch(TOKEN_ENDPOINT, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: form.toString(),
    });
    payload = await response.json();
  } catch (err) {
    console.error("PayTR token request failed", err);
    return { ok: false, error: "Ödeme sağlayıcısına ulaşılamadı. Lütfen tekrar deneyin." };
  }

  if (payload?.status !== "success" || !payload?.token) {
    // Their reason is a developer-facing string in Turkish; log it, and give
    // the buyer something that is about them rather than about our config.
    console.error(`PayTR refused the token request: ${payload?.reason ?? "no reason given"}`);
    return { ok: false, error: "Ödeme başlatılamadı. Lütfen tekrar deneyin." };
  }

  return { ok: true, token: payload.token, amount };
}

export interface Callback {
  ref: string;
  paid: boolean;
  totalAmount: number;
  reason: string;
}

/** Checks that a callback really came from PayTR, and says what it means.
 *
 *  Returns null when the signature does not match, which the route turns into
 *  a refusal — never into an issued licence. This is the only thing standing
 *  between a forged POST and a free Pro licence, so it verifies before it
 *  reads anything else out of the body. */
export async function readCallback(env: Env, form: FormData): Promise<Callback | null> {
  const merchantKey = env.PAYTR_MERCHANT_KEY;
  const merchantSalt = env.PAYTR_MERCHANT_SALT;
  if (!merchantKey || !merchantSalt) return null;

  const ref = String(form.get("merchant_oid") ?? "");
  const status = String(form.get("status") ?? "");
  const totalAmount = String(form.get("total_amount") ?? "");
  const hash = String(form.get("hash") ?? "");
  if (!ref || !status || !hash) return null;

  const expected = b64(await hmacSha256(merchantKey, ref + merchantSalt + status + totalAmount));
  if (!constantTimeEqual(expected, hash)) {
    console.error(`PayTR callback for ${ref} failed its signature check`);
    return null;
  }

  return {
    ref,
    paid: status === "success",
    totalAmount: Number(totalAmount) || 0,
    reason: String(form.get("failed_reason_msg") ?? "") || "Ödeme tamamlanamadı.",
  };
}
