// The licence service: the routes the app calls, the routes a buyer's browser
// calls, and the two a payment processor calls.
//
// It is deliberately not on the path of a translation and never sees one. All
// it does is decide whether a licence key still entitles a machine to have its
// counter lifted, and say so in a token the app can check on its own for up to
// thirty days. If this service is down, every install degrades to the free
// daily allowance rather than to a broken app.
//
// Two processors, split by where the buyer is: PayTR settles in lira for
// Turkey, Paddle acts as merchant of record everywhere else. Which one a
// visitor sees is decided in `/buy` and nowhere else — past that point the
// difference is a `provider` column and a webhook.

import {
  type Cycle,
  type Env,
  USD_PRICE,
  baseUrl,
  isCycle,
  paddleConfigured,
  paddlePriceId,
  paytrConfigured,
  paytrPriceKurus,
  supportEmail,
} from "./env";
import {
  DEFAULT_SEATS,
  type Licence,
  createOrder,
  deliverKey,
  endLicence,
  extendTerm,
  grantToken,
  isLive,
  issueLicence,
  licenceByKey,
  licenceById,
  licenceByProviderRef,
  markOrderFailed,
  markOrderPaid,
  newOrderRef,
  orderByRef,
  refusalFor,
  sweepRevealedKeys,
} from "./licences";
import { handleAdmin } from "./admin";
import { cancelSubscription, customerEmail, cycleFromItems, verifyWebhook } from "./paddle";
import { isLang, pickLang, t, withLang } from "./i18n";
import { type PageContext, accountPage, buyPage, donePage, lira, paytrPage } from "./pages";
import { createCharge, iframeUrl, readCallback } from "./paytr";
import { keys, now, open } from "./tokens";

// -- replies -----------------------------------------------------------------

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

/** The client shows this string to the user verbatim, so it is a sentence
 *  addressed to a person, not a code addressed to a developer. */
const refuse = (message: string, status = 400) => json({ error: message }, status);

const redirect = (location: string) => new Response(null, { status: 303, headers: { location } });

/** The language a page is rendered in, and the URL its switcher links from.
 *  A form post carries its language as a hidden field, which wins over the
 *  cookie: the field is the page the user was actually looking at. */
function pageContext(request: Request, url: URL, formLang?: unknown): PageContext {
  const lang = isLang(formLang) ? formLang : pickLang(request, url);
  return { lang, url };
}

// -- the routes the app calls ------------------------------------------------

async function activate(env: Env, body: any) {
  const key = String(body.key ?? "").trim().toUpperCase();
  const device = String(body.device ?? "").trim();
  if (!key) return refuse("Enter your licence key first.");
  if (!device) return refuse("This copy could not identify the machine it is running on.");

  const licence = await licenceByKey(env, key);
  if (!licence) return refuse("That licence key was not recognised. Check it for typos.", 404);
  if (!isLive(licence)) return refuse(refusalFor(licence, env), 403);

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
          "Open the Account tab on one of them and choose Remove from this device, then try again.",
        409,
      );
    }
    await env.DB.prepare(
      "INSERT INTO seats (licence_id, device, os, app, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?)",
    )
      .bind(licence.id, device, String(body.os ?? ""), String(body.app ?? ""), seen, seen)
      .run();
  }

  return json(await grantToken(env, licence, device));
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

  const licence = await licenceById(env, claims.lic);
  if (!licence) return refuse("This licence no longer exists.", 403);
  if (!isLive(licence)) return refuse(refusalFor(licence, env), 403);

  const seat = await env.DB.prepare("SELECT device FROM seats WHERE licence_id = ? AND device = ?")
    .bind(licence.id, device)
    .first();
  if (!seat) {
    return refuse("This machine is no longer on the licence. Enter your key again to add it.", 403);
  }

  await env.DB.prepare("UPDATE seats SET last_seen = ? WHERE licence_id = ? AND device = ?")
    .bind(now(), licence.id, device)
    .run();

  return json(await grantToken(env, licence, device));
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

// -- buying it ---------------------------------------------------------------

/** Turkey gets PayTR, everywhere else gets Paddle — with an escape hatch.
 *
 *  `?country=` overrides the header, and both variants of the page link to the
 *  other one. Geolocation is a guess: a Turkish customer on a VPN, or someone
 *  living abroad who wants to pay in lira, must not be stuck with the wrong
 *  processor because Cloudflare read an IP address a certain way. */
function inTurkey(request: Request, url: URL): boolean {
  const override = url.searchParams.get("country");
  if (override) return override.toUpperCase() === "TR";
  return (request.headers.get("CF-IPCountry") ?? "").toUpperCase() === "TR";
}

function buy(env: Env, request: Request, url: URL): Response {
  const ctx = pageContext(request, url);
  const turkey = inTurkey(request, url);
  const src = url.searchParams.get("src") ?? "direct";
  const base = baseUrl(env, request);
  const otherUrl = `/buy?country=${turkey ? "XX" : "TR"}&src=${encodeURIComponent(src)}`;

  if (turkey) {
    const monthly = paytrPriceKurus(env, "monthly");
    const yearly = paytrPriceKurus(env, "yearly");
    if (!paytrConfigured(env) || monthly === null || yearly === null) {
      return buyPage({
        ctx,
        turkey,
        configured: false,
        reason: "reasonPaytrUnconfigured",
        monthly: "",
        yearly: "",
        src,
        otherUrl,
      });
    }
    return buyPage({
      ctx,
      turkey,
      configured: true,
      monthly: lira(monthly),
      yearly: lira(yearly),
      src,
      otherUrl,
    });
  }

  if (!paddleConfigured(env)) {
    return buyPage({
      ctx,
      turkey,
      configured: false,
      reason: "reasonPaddleUnconfigured",
      monthly: "",
      yearly: "",
      src,
      otherUrl,
    });
  }
  return buyPage({
    ctx,
    turkey,
    configured: true,
    monthly: USD_PRICE.monthly,
    yearly: USD_PRICE.yearly,
    src,
    otherUrl,
    clientToken: env.PADDLE_CLIENT_TOKEN,
    paddleEnv: env.PADDLE_ENV,
    priceMonthly: paddlePriceId(env, "monthly") ?? "",
    priceYearly: paddlePriceId(env, "yearly") ?? "",
    successUrl: `${base}/done`,
  });
}

async function checkoutPaytr(env: Env, request: Request, url: URL): Promise<Response> {
  const form = await request.formData();
  const ctx = pageContext(request, url, form.get("lang"));
  const s = t(ctx.lang);
  const plain = (text: string, status: number) =>
    new Response(text, { status, headers: { "content-type": "text/plain; charset=utf-8" } });
  // No default. A missing plan is a bug or a hand-made request, and guessing
  // which one someone meant to buy is guessing what to charge them.
  const cycleRaw = String(form.get("cycle") ?? "");
  const email = String(form.get("email") ?? "").trim();

  if (!isCycle(cycleRaw)) return plain(s.invalidPlan, 400);
  if (!email.includes("@")) return plain(s.invalidEmail, 400);
  if (!paytrConfigured(env)) return plain(s.paytrNotConfigured, 503);

  const amount = paytrPriceKurus(env, cycleRaw);
  if (amount === null) return plain(s.priceNotSet, 503);

  const base = baseUrl(env, request);
  const ref = newOrderRef();
  // The order row exists before PayTR is told anything, so the callback can
  // never arrive for an order this service has not heard of.
  await createOrder(env, { ref, provider: "paytr", cycle: cycleRaw, email, amount, currency: "TRY" });

  const charge = await createCharge(env, {
    ref,
    cycle: cycleRaw,
    email,
    userIp: request.headers.get("CF-Connecting-IP") ?? "127.0.0.1",
    okUrl: withLang(`${base}/done?ref=${ref}`, ctx.lang),
    failUrl: withLang(`${base}/done?ref=${ref}`, ctx.lang),
  });

  if (!charge.ok || !charge.token) {
    // The detail goes on the order row for the operator; the buyer gets the
    // sentence in their own language.
    await markOrderFailed(env, ref, charge.error ?? "token request failed");
    return plain(s.paymentCouldNotStart, 502);
  }
  return paytrPage(ctx, iframeUrl(charge.token), ref);
}

async function checkoutPaddle(env: Env, request: Request): Promise<Response> {
  const body: any = await request.json().catch(() => ({}));
  const cycle = String(body.cycle ?? "");
  const email = String(body.email ?? "").trim();

  if (!isCycle(cycle)) return json({ error: "Unknown plan." }, 400);
  if (!paddleConfigured(env)) return json({ error: "Paddle is not configured." }, 503);

  const ref = newOrderRef();
  await createOrder(env, {
    ref,
    provider: "paddle",
    cycle,
    email: email || null,
    // Paddle prices the transaction itself, in the buyer's own currency. The
    // dollar price is recorded for reconciliation, not to charge against.
    amount: cycle === "yearly" ? 2000 : 200,
    currency: "USD",
  });
  return json({ ref });
}

/** What the success page polls. The key is returned for as long as the reveal
 *  window is open, and the unguessable ref is the only thing guarding it. */
async function orderStatus(env: Env, ref: string): Promise<Response> {
  await sweepRevealedKeys(env);
  const order = await orderByRef(env, ref);
  if (!order) return json({ error: "Unknown order." }, 404);
  return json({
    status: order.status,
    key: order.reveal_until && order.reveal_until > now() ? order.licence_key : null,
    failure: order.failure,
  });
}

// -- the routes the processors call ------------------------------------------

/** Turns a paid order into a licence, exactly once.
 *
 *  Both webhooks funnel through here, and both processors retry: PayTR until
 *  it is answered `OK`, Paddle on any non-2xx. So the first thing this does is
 *  ask whether the order has already been paid, because the alternative is
 *  issuing a second licence — and a second charge's worth of seats — for one
 *  payment. */
async function fulfil(
  env: Env,
  ref: string,
  opts: { provider: string; cycle: Cycle; email: string | null; providerRef: string | null },
): Promise<void> {
  const order = await orderByRef(env, ref);
  if (!order) {
    console.error(`fulfilment for an unknown order ${ref}`);
    return;
  }
  if (order.status === "paid") return;

  const { id, key, expiresAt } = await issueLicence(env, {
    provider: opts.provider,
    cycle: opts.cycle,
    email: opts.email ?? order.email,
    providerRef: opts.providerRef,
  });
  await markOrderPaid(env, ref, id, key);
  console.log(`issued licence ${id} for order ${ref} via ${opts.provider}`);
  await deliverKey(env, opts.email ?? order.email, key, opts.cycle, expiresAt);
}

/** PayTR's callback.
 *
 *  It must be answered with the literal string `OK` and nothing else, or PayTR
 *  keeps retrying and eventually flags the merchant account. That includes the
 *  cases where we reject it: a callback whose signature does not verify is
 *  answered `OK` too, because there is nothing PayTR could usefully retry, and
 *  the refusal has already been logged. */
async function paytrWebhook(env: Env, request: Request): Promise<Response> {
  const ok = () => new Response("OK", { headers: { "content-type": "text/plain" } });

  const form = await request.formData().catch(() => null);
  if (!form) return ok();

  const callback = await readCallback(env, form);
  if (!callback) return ok();

  const order = await orderByRef(env, callback.ref);
  if (!order) {
    console.error(`PayTR callback for unknown order ${callback.ref}`);
    return ok();
  }

  if (!callback.paid) {
    await markOrderFailed(env, callback.ref, callback.reason);
    return ok();
  }

  // The amount is checked rather than trusted. A callback that says success
  // for less than the plan costs is either a misconfiguration or an attempt,
  // and both should stop here rather than become a licence.
  if (order.amount !== null && callback.totalAmount < order.amount) {
    console.error(
      `PayTR callback for ${callback.ref} paid ${callback.totalAmount}, expected ${order.amount}`,
    );
    await markOrderFailed(env, callback.ref, "Ödenen tutar plan bedelinden düşük.");
    return ok();
  }

  const cycle: Cycle = isCycle(order.cycle) ? order.cycle : "monthly";
  await fulfil(env, callback.ref, {
    provider: "paytr",
    cycle,
    email: order.email,
    providerRef: callback.ref,
  });
  return ok();
}

async function paddleWebhook(env: Env, request: Request): Promise<Response> {
  const raw = await request.text();
  const event = await verifyWebhook(env, raw, request.headers.get("Paddle-Signature"));
  if (!event) return refuse("Bad signature.", 401);

  const data = event.data ?? {};

  switch (event.eventType) {
    case "transaction.completed": {
      const cycle = cycleFromItems(env, data.items ?? []) ?? "monthly";
      const subscriptionId = data.subscription_id ? String(data.subscription_id) : null;
      const ref = data?.custom_data?.ref ? String(data.custom_data.ref) : null;
      const email = await customerEmail(env, data);

      // A renewal has the same shape as a first payment, minus our ref: the
      // subscription already exists, so this extends it rather than selling
      // another licence.
      if (subscriptionId) {
        const existing = await licenceByProviderRef(env, "paddle", subscriptionId);
        if (existing) {
          const until = await extendTerm(env, existing, cycle);
          console.log(`extended licence ${existing.id} to ${until}`);
          return json({ ok: true });
        }
      }

      if (!ref) {
        console.error("Paddle transaction with no order ref and no known subscription");
        return json({ ok: true, ignored: "no ref" });
      }
      await fulfil(env, ref, { provider: "paddle", cycle, email, providerRef: subscriptionId });
      return json({ ok: true });
    }

    case "subscription.canceled": {
      const licence = await licenceByProviderRef(env, "paddle", String(data.id ?? ""));
      // Cancelling leaves the paid term alone — it has been paid for.
      if (licence) await endLicence(env, licence, "cancelled");
      return json({ ok: true });
    }

    case "adjustment.created": {
      // Refunds arrive as adjustments. Only a refund cuts the term short.
      if (String(data.action ?? "") !== "refund") return json({ ok: true, ignored: data.action });
      const subscriptionId = data.subscription_id ? String(data.subscription_id) : null;
      if (!subscriptionId) return json({ ok: true, ignored: "no subscription" });
      const licence = await licenceByProviderRef(env, "paddle", subscriptionId);
      if (licence) await endLicence(env, licence, "refunded");
      return json({ ok: true });
    }

    default:
      return json({ ok: true, ignored: event.eventType });
  }
}

// -- managing it -------------------------------------------------------------

async function accountView(
  env: Env,
  ctx: PageContext,
  licence: Licence,
  key: string,
  message?: string,
  error?: string,
) {
  const { count } = (await env.DB.prepare("SELECT COUNT(*) AS count FROM seats WHERE licence_id = ?")
    .bind(licence.id)
    .first<{ count: number }>())!;

  return accountPage(
    ctx,
    {
      key,
      plan: licence.plan === "pro" ? "bubbleTranslate Pro" : licence.plan,
      cycle: licence.cycle,
      status: isLive(licence) ? licence.status : "expired",
      renews: licence.renews_at,
      seats: count,
      seatLimit: licence.seat_limit,
      provider: licence.provider,
      // PayTR licences are fixed-term and do not recur, so there is nothing to
      // cancel — see the note at the top of `paytr.ts`.
      cancellable: licence.provider === "paddle" && licence.status === "active",
      message,
      error,
    },
    supportEmail(env),
  );
}

async function account(env: Env, request: Request, url: URL): Promise<Response> {
  const form = await request.formData();
  const ctx = pageContext(request, url, form.get("lang"));
  const s = t(ctx.lang);
  const key = String(form.get("key") ?? "").trim().toUpperCase();
  if (!key) return accountPage(ctx, null, supportEmail(env), s.enterYourKey);

  const licence = await licenceByKey(env, key);
  if (!licence) {
    return accountPage(ctx, null, supportEmail(env), s.keyNotRecognised);
  }
  return accountView(env, ctx, licence, key);
}

async function accountCancel(env: Env, request: Request, url: URL): Promise<Response> {
  const form = await request.formData();
  const ctx = pageContext(request, url, form.get("lang"));
  const s = t(ctx.lang);
  const key = String(form.get("key") ?? "").trim().toUpperCase();
  const licence = await licenceByKey(env, key);
  if (!licence) {
    return accountPage(ctx, null, supportEmail(env), s.keyNotRecognised);
  }
  if (licence.provider !== "paddle" || !licence.provider_ref) {
    return accountView(env, ctx, licence, key, undefined, s.noRecurringCharge);
  }

  const failure = await cancelSubscription(env, licence.provider_ref);
  if (failure) return accountView(env, ctx, licence, key, undefined, s[failure]);

  // Paddle will also send `subscription.canceled`; recording it now means the
  // page the user is about to see tells the truth without waiting for it.
  await endLicence(env, licence, "cancelled");
  const updated = (await licenceById(env, licence.id)) ?? licence;
  return accountView(env, ctx, updated, key, s.cancelled(updated.renews_at ?? s.endOfPaidPeriod));
}

// -- development -------------------------------------------------------------

async function devIssue(env: Env, body: any) {
  const cycle: Cycle = isCycle(body.cycle) ? body.cycle : "yearly";
  const { id, key, expiresAt } = await issueLicence(env, {
    provider: String(body.provider ?? "dev"),
    cycle,
    email: body.email ?? null,
    providerRef: body.provider_ref ?? null,
    seats: Number(body.seats) || DEFAULT_SEATS,
    termSeconds: Number(body.term_seconds) || undefined,
  });
  return json({ id, key, cycle, expires_at: expiresAt });
}

// -- the router --------------------------------------------------------------

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const { pathname } = url;
    const dev = Boolean(env.DEV_MODE);

    try {
      // The operator's panel owns everything under /admin, including its own
      // authentication. It answers null for anything else, so this cannot
      // shadow a route below it.
      const admin = await handleAdmin(env, request, url);
      if (admin) return admin;

      if (request.method === "GET") {
        if (pathname === "/buy") return buy(env, request, url);
        if (pathname === "/account") {
          return accountPage(pageContext(request, url), null, supportEmail(env));
        }
        if (pathname === "/done") {
          const ctx = pageContext(request, url);
          const ref = url.searchParams.get("ref") ?? "";
          if (!ref) return redirect(withLang("/buy", ctx.lang));
          return donePage(ctx, ref, supportEmail(env));
        }
        if (pathname.startsWith("/v1/order/")) {
          return orderStatus(env, pathname.slice("/v1/order/".length));
        }
        if (pathname === "/v1/pubkey" && dev) {
          return json({ public_key_hex: (await keys(env)).publicHex });
        }
        return new Response("Not found.", { status: 404 });
      }

      if (request.method !== "POST") return refuse("Not found.", 404);

      // Webhooks and browser form posts read their own bodies — one is signed
      // over its raw bytes, the others are form-encoded — so they are routed
      // before anything tries to parse the body as JSON.
      switch (pathname) {
        case "/webhooks/paytr":
          return await paytrWebhook(env, request);
        case "/webhooks/paddle":
          return await paddleWebhook(env, request);
        case "/checkout/paytr":
          return await checkoutPaytr(env, request, url);
        case "/checkout/paddle":
          return await checkoutPaddle(env, request);
        case "/account":
          return await account(env, request, url);
        case "/account/cancel":
          return await accountCancel(env, request, url);
      }

      const body = await request.json().catch(() => ({}));
      switch (pathname) {
        case "/v1/activate":
          return await activate(env, body);
        case "/v1/refresh":
          return await refresh(env, body);
        case "/v1/deactivate":
          return await deactivate(env, body);
        case "/v1/dev/issue":
          return dev ? await devIssue(env, body) : refuse("Not found.", 404);
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
