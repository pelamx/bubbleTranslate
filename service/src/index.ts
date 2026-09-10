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
  paddleEnvOrThrow,
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
import {
  type CancelFailure,
  cancelSubscription,
  customerEmail,
  cycleFromItems,
  portalSession,
  verifyWebhook,
} from "./paddle";
import {
  grantsAccess,
  mirrorCustomer,
  mirrorSubscription,
  noteCustomer,
  occurredAt,
  subscriptionById,
} from "./mirror";
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

/** The visitor's country, or undefined when we genuinely do not know.
 *
 *  Three different things mean "unknown" here and none of them is a country.
 *  `/buy?country=XX` is this app's own sentinel for "not Turkey", set by the
 *  link between the two buy pages. Cloudflare sends `XX` when it cannot place
 *  an address and `T1` when the request came out of Tor. Passing any of them
 *  to Paddle as a country code is an error; omitting the address instead lets
 *  Paddle geolocate the IP, which is what it does best. */
function visitorCountry(request: Request, url: URL): string | undefined {
  const raw = (url.searchParams.get("country") ?? request.headers.get("CF-IPCountry") ?? "")
    .trim()
    .toUpperCase();
  if (!/^[A-Z]{2}$/.test(raw)) return undefined;
  if (raw === "XX" || raw === "T1") return undefined;
  return raw;
}

function buy(env: Env, request: Request, url: URL): Response {
  const ctx = pageContext(request, url);
  const wantsTurkish = inTurkey(request, url);
  const src = url.searchParams.get("src") ?? "direct";
  const base = baseUrl(env, request);

  const monthlyKurus = paytrPriceKurus(env, "monthly");
  const yearlyKurus = paytrPriceKurus(env, "yearly");
  const paytrReady =
    paytrConfigured(env) && monthlyKurus !== null && yearlyKurus !== null;

  // A visitor whose own processor is not set up is sent to the other one
  // rather than to a dead end. Turkey is routed to PayTR because lira and a
  // local card are what people there expect -- but "we would rather bill you
  // in lira" is not a reason to refuse a customer who is holding out a card,
  // and this page used to do exactly that: it told Turkish visitors that
  // checkout was unavailable while Paddle sat configured and idle.
  //
  // It matters for payment links too. Paddle sends customers to the account's
  // default payment link, and that link has to open a Paddle checkout for
  // whoever follows it, wherever they happen to be.
  const serveTurkish = wantsTurkish && paytrReady;
  const otherUrl = `/buy?country=${serveTurkish ? "XX" : "TR"}&src=${encodeURIComponent(src)}`;

  if (serveTurkish) {
    return buyPage({
      ctx,
      turkey: true,
      configured: true,
      monthly: lira(monthlyKurus!),
      yearly: lira(yearlyKurus!),
      src,
      otherUrl,
    });
  }

  if (!paddleConfigured(env)) {
    return buyPage({
      ctx,
      turkey: wantsTurkish,
      configured: false,
      reason: wantsTurkish ? "reasonPaytrUnconfigured" : "reasonPaddleUnconfigured",
      monthly: "",
      yearly: "",
      src,
      otherUrl,
    });
  }
  return buyPage({
    ctx,
    turkey: false,
    configured: true,
    monthly: USD_PRICE.monthly,
    yearly: USD_PRICE.yearly,
    src,
    otherUrl,
    clientToken: env.PADDLE_CLIENT_TOKEN,
    paddleEnv: paddleEnvOrThrow(env),
    priceMonthly: paddlePriceId(env, "monthly") ?? "",
    priceYearly: paddlePriceId(env, "yearly") ?? "",
    country: visitorCountry(request, url),
    successUrl: `${base}/welcome`,
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
  // Paddle's own idea of when this happened. Deliveries are at-least-once and
  // unordered, so every mirror write is guarded by it: an old payload arriving
  // late must not overwrite a newer status.
  const eventAt = occurredAt(event);

  switch (event.eventType) {
    case "transaction.completed": {
      const cycle = cycleFromItems(env, data.items ?? []) ?? "monthly";
      const subscriptionId = data.subscription_id ? String(data.subscription_id) : null;
      const ref = data?.custom_data?.ref ? String(data.custom_data.ref) : null;
      const email = await customerEmail(env, data);
      await noteCustomer(env, data?.customer_id ? String(data.customer_id) : null, email);

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

    case "subscription.created":
    case "subscription.updated": {
      // The mirror takes every one of these; the licence is only moved by a
      // status that actually stops the entitlement. A `scheduled_change` to
      // cancel is not one: it says what Paddle will do at the end of a period
      // the customer has already paid for, and acting on it now would take
      // back time they bought. `subscription.canceled` is what does that.
      await mirrorSubscription(env, data, eventAt);
      return json({ ok: true });
    }

    case "subscription.canceled": {
      await mirrorSubscription(env, data, eventAt);
      const licence = await licenceByProviderRef(env, "paddle", String(data.id ?? ""));
      // Cancelling leaves the paid term alone — it has been paid for.
      if (licence) await endLicence(env, licence, "cancelled");
      return json({ ok: true });
    }

    case "customer.created":
    case "customer.updated": {
      // One write for both: the second delivery of a create and the first of
      // an update are indistinguishable, and should leave the same row.
      await mirrorCustomer(env, data, eventAt);
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

  // What Paddle believes, if a webhook has told us. It only ever decorates
  // this page -- the licence row is what entitles anyone to anything, and a
  // mirror that is empty (an old licence, a webhook not yet delivered) must
  // not make the page refuse to work.
  const subscription =
    licence.provider === "paddle" && licence.provider_ref
      ? await subscriptionById(env, licence.provider_ref)
      : null;

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
      cancellable:
        licence.provider === "paddle" &&
        licence.status === "active" &&
        // A subscription Paddle has already stopped billing has nothing left
        // to cancel; the button would only produce an API error.
        (!subscription || grantsAccess(subscription)),
      // The portal needs a customer id, and the only place one comes from is
      // a webhook. No mirror row, no button -- rather than a button that
      // fails after the click.
      portal: Boolean(subscription?.customer_id),
      // A scheduled cancellation is a future intention, not a current state.
      // It is said out loud here precisely because it revokes nothing yet.
      scheduledCancelAt:
        subscription?.scheduled_change_action === "cancel"
          ? (subscription.scheduled_change_at ?? "").slice(0, 10) || null
          : null,
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

/** Opens the Paddle-hosted customer portal, where the customer updates a card,
 *  reads invoices or cancels.
 *
 *  Authentication first, and the licence key is the credential: this service
 *  has no accounts and no cookies, so holding the key is what proves the
 *  visitor is the customer. The Paddle customer id is then resolved from the
 *  mirror on this side. Nothing the browser sent names a customer -- a form
 *  field holding a `ctm_...` would be an invitation to read a stranger's
 *  billing history.
 *
 *  The session URL is redirected to and never stored. Paddle expires these
 *  quickly, and a cached one is exactly that stranger's billing history. */
async function accountPortal(env: Env, request: Request, url: URL): Promise<Response> {
  const form = await request.formData();
  const ctx = pageContext(request, url, form.get("lang"));
  const s = t(ctx.lang);
  const key = String(form.get("key") ?? "").trim().toUpperCase();

  const licence = key ? await licenceByKey(env, key) : null;
  if (!licence) return accountPage(ctx, null, supportEmail(env), s.keyNotRecognised);

  if (licence.provider !== "paddle" || !licence.provider_ref) {
    return accountView(env, ctx, licence, key, undefined, s.portalNotAvailable);
  }

  const subscription = await subscriptionById(env, licence.provider_ref);
  if (!subscription?.customer_id) {
    return accountView(env, ctx, licence, key, undefined, s.portalNotAvailable);
  }

  const result = await portalSession(env, subscription.customer_id, [licence.provider_ref]);
  if (!result.startsWith("http")) {
    return accountView(env, ctx, licence, key, undefined, s[result as CancelFailure]);
  }
  return new Response(null, { status: 303, headers: { location: result } });
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
        // /welcome is where Paddle returns the buyer, and /done is where PayTR
        // does. They are the same page: it polls for the licence key using the
        // ref in the query string, so the ref has to survive the rename.
        if (pathname === "/done" || pathname === "/welcome") {
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
        case "/account/portal":
          return await accountPortal(env, request, url);
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
