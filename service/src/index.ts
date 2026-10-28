// The licence service: the routes the app calls, the routes a buyer's browser
// calls, and the two a payment processor calls.
//
// It is deliberately not on the path of a translation, with two exceptions:
// the DeepL fallback in `deepl.ts`, asked only once Google and MyMemory have
// both failed, and a single word translated in its sentence by Claude Haiku in
// `word.ts`, for copies without an Anthropic key of their own. If either is
// down the app translates without it. Otherwise all it does is decide whether a licence key still entitles
// a machine to have its counter lifted, and say so in a token the app can
// check on its own for up to thirty days. If this service is down, every install degrades to the free
// daily allowance rather than to a broken app.
//
// Paddle is the single processor: it acts as merchant of record everywhere,
// so it owns VAT, sales tax and invoicing. Buying is decided in `/buy`; past
// that point the difference is a `provider` column and a webhook.

import {
  type Cycle,
  type Env,
  USD_AMOUNT,
  USD_PRICE,
  baseUrl,
  isCycle,
  paddleConfigured,
  paddleEnvOrThrow,
  paddlePriceId,
  supportEmail,
} from "./env";
import {
  DEFAULT_SEATS,
  type Licence,
  claimOrderAndIssue,
  claimTransaction,
  TOKEN_TTL,
  releaseTransaction,
  createOrder,
  deliverKey,
  endLicence,
  extendTerm,
  grantToken,
  isCancelReason,
  isLive,
  issueLicence,
  licenceByKey,
  licenceById,
  licenceByProviderRef,
  newOrderRef,
  orderByRef,
  refusalFor,
  sweepRevealedKeys,
} from "./licences";
import { handleAdmin } from "./admin";
import { freeDailyTranslations, recordMetrics, remoteSettings } from "./observability";
import { deeplRelay } from "./deepl";
import { wordRelay } from "./word";
import { pushConversions, type ClickIds } from "./ads";
import { sendAlerts } from "./alerts";
import { sendWeeklyReport } from "./report";
import {
  type CancelFailure,
  type Verified,
  cancelSubscription,
  customerCountry,
  customerEmail,
  cycleFromItems,
  fromPaddle,
  portalSession,
  verifyWebhook,
} from "./paddle";
import {
  grantsAccess,
  mirrorAdjustment,
  mirrorCustomer,
  mirrorSubscription,
  noteCustomer,
  occurredAt,
  subscriptionById,
} from "./mirror";
import { isLang, pickLang, t, withLang } from "./i18n";
import { SITE, type PageContext, accountPage, buyPage, donePage } from "./pages";
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

/** The daily "in use" ping. Upserts one row per install; answers 204 whatever
 *  it was sent, because the app ignores the reply and a junk ping is not worth
 *  an error path. */
async function ping(env: Env, body: any, request: Request) {
  const install = String(body.install ?? "");
  if (!/^[0-9a-f]{32}$/.test(install)) return new Response(null, { status: 204 });
  const plan = body.plan === "pro" ? "pro" : "free";
  const t = now();
  // The country Cloudflare already worked out from the address, and only
  // that: the address itself is not kept. Absent in tests and local dev.
  const cf = (request as { cf?: { country?: unknown } }).cf;
  const country =
    typeof cf?.country === "string" && /^[A-Z0-9]{2}$/.test(cf.country) ? cf.country : null;
  const os = String(body.os ?? "").slice(0, 16);
  const app = String(body.app ?? "").slice(0, 32);
  // Before the row is overwritten: a known install reporting a different
  // version from last time has been updated, which is the only moment that
  // can be seen.
  await env.DB.prepare(
    `INSERT INTO updates (install, os, from_app, to_app, at)
     SELECT install, ?2, app, ?3, ?4 FROM installs
      WHERE install = ?1 AND app IS NOT NULL AND app <> '' AND ?3 <> '' AND app <> ?3`,
  )
    .bind(install, os, app, t)
    .run();
  const row = await env.DB.prepare(
    `INSERT INTO installs (install, os, app, plan, first_seen, last_seen, country, alive_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?5)
     ON CONFLICT (install) DO UPDATE SET os = ?2, app = ?3, plan = ?4, last_seen = ?5,
       alive_at = ?5, country = COALESCE(?6, country)
     RETURNING first_seen, source`,
  )
    .bind(install, os, app, plan, t, country)
    .first<{ first_seen: number; source: string | null }>();
  // Only a brand-new install is matched, and only once: an old one already
  // has its answer, or predates the matching and stays unknown.
  // A store copy was never a website download, so it is not matched to a
  // download click: that would take the click away from the install that
  // really came from it.
  const channel = body.channel === "store" ? "store" : body.channel === "download" ? "download" : null;
  if (row && row.first_seen === t && row.source === null) {
    if (channel === "store") {
      await env.DB.prepare("UPDATE installs SET source = 'store' WHERE install = ?").bind(install).run();
    } else {
      await attributeInstall(env, install, os, country, t);
    }
  }
  if (channel) {
    // Its own statement, and allowed to fail: the column is newer than the
    // table, and a database without it must not lose the install count.
    try {
      await env.DB.prepare("UPDATE installs SET channel = ? WHERE install = ?").bind(channel, install).run();
    } catch (err) {
      console.error("could not record the channel", err);
    }
  }
  // Sent by the app the first time in a day the free allowance runs out. The
  // first time is kept, and each new day adds one, so a copy restarted twice
  // on a capped day still counts that day once.
  if (body.capped === true) {
    await env.DB.prepare(
      `UPDATE installs SET
         first_capped = COALESCE(first_capped, ?2),
         capped_days = capped_days + CASE
           WHEN last_capped IS NULL OR date(last_capped, 'unixepoch') <> date(?2, 'unixepoch')
           THEN 1 ELSE 0 END,
         last_capped = ?2
       WHERE install = ?1`,
    )
      .bind(install, t)
      .run();
  }
  await recordProviderHealth(env, body.providers, t);
  // The app keeps its daily counts until a ping is answered with success, so
  // a failure to store them is the one thing this route does not hide.
  const dailyKept = await recordDaily(env, install, os, app, body.daily, t);
  const metricsKept = await recordMetrics(env, install, os, channel ?? "download", body.metrics, t);
  return new Response(null, { status: dailyKept && metricsKept ? 204 : 503 });
}

/** Run once a day by the cron, not on every five-minute ping. The privacy
 *  policy promises that an install silent for 13 months is forgotten; an order
 *  nobody paid for is kept for the panel's 30-day funnel and then dropped, so
 *  a script posting to /checkout/paddle cannot grow the table for ever. */
export async function forgetOld(env: Env) {
  const t = now();
  const forgotten = new Date((t - 396 * 86_400) * 1000).toISOString().slice(0, 10);
  await env.DB.batch([
    env.DB.prepare("DELETE FROM installs WHERE last_seen < ?").bind(t - 396 * 86_400),
    env.DB.prepare("DELETE FROM updates WHERE at < ?").bind(t - 396 * 86_400),
    env.DB.prepare("DELETE FROM claude_usage WHERE day < ?").bind(forgotten),
    env.DB.prepare("DELETE FROM screen_usage WHERE day < ?").bind(forgotten),
    env.DB.prepare("DELETE FROM translate_usage WHERE day < ?").bind(forgotten),
    env.DB.prepare("DELETE FROM orders WHERE status != 'paid' AND created_at < ?").bind(t - 90 * 86_400),
  ]);
}

/** "Still open", sent every few minutes by a running copy. Only moves
 *  `alive_at` on an install the daily ping already created: it carries the
 *  install id and nothing else, so there is nothing more to record, and an
 *  unknown id is not worth a row. */
async function alive(env: Env, body: any) {
  const install = String(body.install ?? "");
  if (/^[0-9a-f]{32}$/.test(install)) {
    await env.DB.prepare("UPDATE installs SET alive_at = ?1 WHERE install = ?2")
      .bind(now(), install)
      .run();
  }
  return new Response(null, { status: 204 });
}

/** How far back a download click can be and still be the one a first ping
 *  came from: long enough for someone who downloads in the evening and opens
 *  it after the weekend, short enough not to claim last month's visitor. */
const ATTRIBUTION_WINDOW = 3 * 86_400;

/** Gives a new install the source of the website download it most likely
 *  came from: the latest unclaimed click for the same OS from the same
 *  country, within the window. The click is claimed so the next install
 *  cannot take it too. No click means it came some other way -- the GitHub
 *  page, the README, a link shared directly -- and is recorded as such, so
 *  "unmatched" is a number rather than a gap. */
async function attributeInstall(
  env: Env,
  install: string,
  os: string,
  country: string | null,
  t: number,
): Promise<void> {
  try {
    const click = await env.DB.prepare(
      `UPDATE download_clicks SET install = ?1
        WHERE id = (SELECT id FROM download_clicks
                     WHERE install IS NULL AND os = ?2 AND country IS ?3
                       AND at BETWEEN ?4 AND ?5
                     ORDER BY at DESC LIMIT 1)
        RETURNING src`,
    )
      .bind(install, os, country, t - ATTRIBUTION_WINDOW, t)
      .first<{ src: string }>();
    await env.DB.prepare("UPDATE installs SET source = ? WHERE install = ?")
      .bind(click?.src ?? "unmatched", install)
      .run();
  } catch (err) {
    // A ping is never failed over a guess about where it came from.
    console.error("could not attribute an install", err);
  }
}

/** The tag a download click arrives with, as the panel will show it. The
 *  website sends a link's `utm_source` or `ref`, or the host of the site that
 *  sent the visitor; anything that is not a short plain word is `other`. */
export function cleanSource(raw: unknown): string {
  const s = String(raw ?? "").trim().toLowerCase().replace(/^www\./, "");
  if (!s) return "direct";
  return /^[a-z0-9][a-z0-9._-]{0,39}$/.test(s) ? s : "other";
}

/** One press of a download button on the website, sent with `sendBeacon` so
 *  the download itself never waits on this service. Answers 204 whatever it
 *  is sent, like the ping: the page cannot read the reply anyway. */
async function downloadClick(env: Env, body: any, request: Request): Promise<Response> {
  const os = String(body.os ?? "");
  if (!["macos", "windows", "linux"].includes(os)) return new Response(null, { status: 204 });
  try {
    const t = now();
    const cf = (request as { cf?: { country?: unknown } }).cf;
    const country =
      typeof cf?.country === "string" && /^[A-Z0-9]{2}$/.test(cf.country) ? cf.country : null;
    await env.DB.batch([
      env.DB.prepare("INSERT INTO download_clicks (at, os, src, country) VALUES (?, ?, ?, ?)").bind(
        t,
        os,
        cleanSource(body.src),
        country,
      ),
      env.DB.prepare("DELETE FROM download_clicks WHERE at < ?").bind(t - 90 * 86_400),
    ]);
  } catch (err) {
    console.error("could not log a download click", err);
  }
  return new Response(null, { status: 204 });
}

/// Adds one install's per-backend tallies into the day's totals, as copies
/// released up to 0.4.18 send them: today's so far, filed under the day the
/// ping arrived. Later copies send finished days through `recordDaily`.
///
/// Anything unrecognised is dropped rather than rejected: this rides along on a
/// ping whose answer is ignored, so a malformed field must never cost the caller
/// the install count it came for. Provider names are capped and counts clamped,
/// because the body is whatever was posted.
async function recordProviderHealth(env: Env, reported: unknown, t: number) {
  if (!reported || typeof reported !== "object" || Array.isArray(reported)) return;
  const day = new Date(t * 1000).toISOString().slice(0, 10);

  const rows = Object.entries(reported as Record<string, unknown>)
    .slice(0, 16)
    .flatMap(([name, tally]) => {
      if (!tally || typeof tally !== "object") return [];
      const provider = name.trim().slice(0, 24);
      if (!provider) return [];
      const count = (value: unknown) => {
        const n = Number((tally as Record<string, unknown>)[value as string]);
        return Number.isFinite(n) && n > 0 ? Math.min(Math.floor(n), 1_000_000) : 0;
      };
      const ok = count("ok");
      const failed = count("failed");
      if (ok === 0 && failed === 0) return [];
      return [{ provider, ok, failed }];
    });
  if (rows.length === 0) return;

  await env.DB.batch(
    rows.map((row) =>
      env.DB.prepare(
        `INSERT INTO provider_health (day, provider, ok, failed, reports)
         VALUES (?1, ?2, ?3, ?4, 1)
         ON CONFLICT (day, provider) DO UPDATE SET
           ok      = ok + ?3,
           failed  = failed + ?4,
           reports = reports + 1`,
      ).bind(day, row.provider, row.ok, row.failed),
    ),
  );
}

/// Adds one install's day-by-day counts (see `outbox` in the client): which
/// backends answered, how many translations it made on which plan, what it
/// asked of Claude on its own key, and how many screen readings it made or was
/// refused. Each day is the install's own local
/// date.
///
/// The app keeps a day's counts until a ping has delivered them, so a day
/// usually arrives once, the morning after; a ping that was received but whose
/// answer was lost sends it again, and that day is counted twice. Nothing in
/// it is trusted: days outside the last month, unknown sections, odd names and
/// absurd counts are dropped or clamped, and a malformed entry must never cost
/// the ping the install count it came for. False only when the rows could not
/// be written, so the app keeps them for the next ping.
async function recordDaily(
  env: Env,
  install: string,
  os: string,
  app: string,
  reported: unknown,
  t: number,
): Promise<boolean> {
  if (!Array.isArray(reported)) return true;
  const today = Math.floor(t / 86_400);
  const count = (counts: any, field: string, cap: number) => {
    const n = Number(counts?.[field]);
    return Number.isFinite(n) && n > 0 ? Math.min(Math.floor(n), cap) : 0;
  };
  const statements: D1PreparedStatement[] = [];
  for (const e of reported.slice(0, 128) as any[]) {
    // A day ahead is a machine east of UTC, not one from the future.
    const dayNumber = Number(e?.day);
    if (!Number.isInteger(dayNumber) || dayNumber < today - 31 || dayNumber > today + 1) continue;
    const day = new Date(dayNumber * 86_400_000).toISOString().slice(0, 10);
    const name = String(e?.name ?? "").trim();
    const counts = e?.counts;
    switch (e?.section) {
      case "claude": {
        if (!/^[a-z0-9.-]{1,48}$/.test(name)) continue;
        const c = {
          requests: count(counts, "requests", 100_000),
          failed: count(counts, "failed", 100_000),
          input: count(counts, "input", 1_000_000_000),
          output: count(counts, "output", 1_000_000_000),
          cache_read: count(counts, "cache_read", 1_000_000_000),
          cache_write: count(counts, "cache_write", 1_000_000_000),
        };
        if (!c.requests && !c.failed) continue;
        statements.push(
          env.DB.prepare(
            `INSERT INTO claude_usage (day, install, model, os, requests, failed, input, output, cache_read, cache_write)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (day, install, model) DO UPDATE SET
               os = ?4,
               requests = requests + ?5, failed = failed + ?6,
               input = input + ?7, output = output + ?8,
               cache_read = cache_read + ?9, cache_write = cache_write + ?10`,
          ).bind(day, install, name, os, c.requests, c.failed, c.input, c.output, c.cache_read, c.cache_write),
        );
        break;
      }
      case "providers": {
        const provider = name.slice(0, 24);
        const ok = count(counts, "ok", 1_000_000);
        const failed = count(counts, "failed", 1_000_000);
        if (!provider || (!ok && !failed)) continue;
        statements.push(
          env.DB.prepare(
            `INSERT INTO provider_health (day, provider, ok, failed, reports)
             VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT (day, provider) DO UPDATE SET
               ok = ok + ?3, failed = failed + ?4, reports = reports + 1`,
          ).bind(day, provider, ok, failed),
        );
        break;
      }
      case "translate": {
        if (name !== "free" && name !== "pro") continue;
        const done = count(counts, "done", 100_000);
        const charged = count(counts, "charged", 100_000);
        const capped = count(counts, "capped", 100_000);
        if (!done && !charged && !capped) continue;
        statements.push(
          env.DB.prepare(
            `INSERT INTO translate_usage (day, install, plan, os, app, done, charged, capped)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (day, install, plan) DO UPDATE SET
               os = ?4, app = COALESCE(?5, app),
               done = done + ?6, charged = charged + ?7, capped = capped + ?8`,
          ).bind(day, install, name, os, app || null, done, charged, capped),
        );
        break;
      }
      case "screen": {
        if (name !== "box" && name !== "page") continue;
        const done = count(counts, "done", 100_000);
        const capped = count(counts, "capped", 100_000);
        if (!done && !capped) continue;
        statements.push(
          env.DB.prepare(
            `INSERT INTO screen_usage (day, install, kind, os, done, capped)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (day, install, kind) DO UPDATE SET
               os = ?4, done = done + ?5, capped = capped + ?6`,
          ).bind(day, install, name, os, done, capped),
        );
        break;
      }
    }
  }
  if (statements.length === 0) return true;
  try {
    await env.DB.batch(statements);
    return true;
  } catch (err) {
    console.error("could not record the daily counts", err);
    return false;
  }
}

/** New machines a licence may take in a month beyond its seat limit. */
const NEW_MACHINE_ALLOWANCE = 3;

async function activate(env: Env, body: any) {
  const key = String(body.key ?? "").trim().toUpperCase();
  const device = String(body.device ?? "").trim();
  if (!key) return refuse("Enter your licence key first.");
  if (!device) return refuse("This copy could not identify the machine it is running on.");
  // The device goes into every token, and all three into the database; a
  // real client sends a hash, an OS name and a version.
  if (device.length > 128) return refuse("This copy could not identify the machine it is running on.");
  const os = String(body.os ?? "").slice(0, 32);
  const app = String(body.app ?? "").slice(0, 32);

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
    // New machines are capped per month, not only at once: a removed seat's
    // token keeps working offline until it expires, so without this, adding
    // and removing machines in turn would spread one key over any number of
    // them. The allowance above the seat limit is for real moves — a new
    // laptop, a reinstall — and support can reset it by freeing the seats.
    const recent = await env.DB.prepare(
      "SELECT COUNT(*) AS n FROM activations WHERE licence_id = ? AND device != ? AND at > ?",
    )
      .bind(licence.id, device, seen - TOKEN_TTL)
      .first<{ n: number }>();
    if ((recent?.n ?? 0) >= licence.seat_limit + NEW_MACHINE_ALLOWANCE) {
      return refuse(
        "This licence has been added to too many new machines this month. " +
          "Write to support and we will reset it.",
        429,
      );
    }

    // One statement decides, rather than count-then-insert: two activations
    // racing for the last free slot would both pass a separate count check
    // and both insert, and the seat limit would be whatever the racing made
    // of it. Here the count and the insert are the same write, so only one
    // of them can win, and `meta.changes` says which.
    const inserted = await env.DB.prepare(
      `INSERT INTO seats (licence_id, device, os, app, first_seen, last_seen)
       SELECT ?1, ?2, ?3, ?4, ?5, ?6
       WHERE (SELECT COUNT(*) FROM seats WHERE licence_id = ?1) < ?7`,
    )
      .bind(
        licence.id,
        device,
        os,
        app,
        seen,
        seen,
        licence.seat_limit,
      )
      .run();
    if (!inserted.meta.changes) {
      return refuse(
        `This licence is already in use on ${licence.seat_limit} machines. ` +
          "Open the Account tab on one of them and choose Remove from this device, then try again.",
        409,
      );
    }
    await env.DB.prepare(
      `INSERT INTO activations (licence_id, device, at) VALUES (?, ?, ?)
       ON CONFLICT (licence_id, device) DO UPDATE SET at = excluded.at`,
    )
      .bind(licence.id, device, seen)
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
  // A token is a bearer credential for exactly one machine, and releasing a
  // seat is something only that machine's token may do. Honoring a
  // `body.device` naming a different seat would let one device free another's
  // slot on a licence it merely holds a token for.
  const device = String(body.device ?? claims.dev);
  if (device !== claims.dev) return json({ ok: false }, 403);
  await env.DB.prepare("DELETE FROM seats WHERE licence_id = ? AND device = ?")
    .bind(claims.lic, device)
    .run();
  return json({ ok: true });
}

// -- buying it ---------------------------------------------------------------

/** The visitor's country, or undefined when we genuinely do not know.
 *
 *  Three different things mean "unknown" here and none of them is a country.
 *  Cloudflare sends `XX` when it cannot place an address and `T1` when the
 *  request came out of Tor. Passing either to Paddle as a country code is an
 *  error; omitting the address instead lets Paddle geolocate the IP, which is
 *  what it does best. */
function visitorCountry(request: Request, url: URL): string | undefined {
  const raw = (url.searchParams.get("country") ?? request.headers.get("CF-IPCountry") ?? "")
    .trim()
    .toUpperCase();
  if (!/^[A-Z]{2}$/.test(raw)) return undefined;
  if (raw === "XX" || raw === "T1") return undefined;
  return raw;
}

async function buy(env: Env, request: Request, url: URL): Promise<Response> {
  const ctx = pageContext(request, url);
  const src = url.searchParams.get("src") ?? "direct";
  const base = baseUrl(env, request);
  await logBuyVisit(env, request, src, ctx.lang);
  const freeDaily = await freeDailyTranslations(env);

  if (!paddleConfigured(env)) {
    return buyPage({
      ctx,
      configured: false,
      reason: "reasonPaddleUnconfigured",
      monthly: "",
      yearly: "",
      src,
      freeDaily,
    });
  }
  return buyPage({
    ctx,
    configured: true,
    monthly: USD_PRICE.monthly,
    yearly: USD_PRICE.yearly,
    src,
    clientToken: env.PADDLE_CLIENT_TOKEN,
    paddleEnv: paddleEnvOrThrow(env),
    priceMonthly: paddlePriceId(env, "monthly") ?? "",
    priceYearly: paddlePriceId(env, "yearly") ?? "",
    country: visitorCountry(request, url),
    successUrl: `${base}/welcome`,
    freeDaily,
  });
}

/** Where a buy page link was. The app sends `bubble`, `bubble-low` or
 *  `window`; anything else is a link somebody wrote, and the panel only needs
 *  to know it was not the app. The same value goes on the visit and on the
 *  order, so the two can be lined up per source. */
export function buySource(raw: unknown): string {
  const src = String(raw ?? "").trim() || "direct";
  return /^[a-z][a-z-]{0,15}$/.test(src) ? src : "other";
}

/** One row in `buy_visits` for the admin panel's funnel. Never fails the
 *  page: a buyer must not be turned away because a counter could not be
 *  written. */
async function logBuyVisit(env: Env, request: Request, src: string, lang: string): Promise<void> {
  try {
    const t = now();
    const cf = (request as { cf?: { country?: unknown } }).cf;
    const country =
      typeof cf?.country === "string" && /^[A-Z]{2}$/.test(cf.country) ? cf.country : null;
    const from = buySource(src);
    await env.DB.batch([
      env.DB.prepare(`INSERT INTO buy_visits (at, src, lang, country) VALUES (?, ?, ?, ?)`).bind(
        t,
        from,
        lang,
        country,
      ),
      env.DB.prepare("DELETE FROM buy_visits WHERE at < ?").bind(t - 90 * 86_400),
    ]);
  } catch (err) {
    console.error("could not log a buy page visit", err);
  }
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
    amount: USD_AMOUNT[cycle] * 100,
    currency: "USD",
    src: buySource(body.src),
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

/** The ad click ids Paddle carried through in custom_data. Only the keys the
 *  networks actually match on are kept, and each is a plain string or absent --
 *  custom_data is attacker-influenced in principle, so nothing here is
 *  trusted beyond being copied into an outbound conversion. */
function readClickIds(customData: any): ClickIds {
  const out: ClickIds = {};
  if (!customData || typeof customData !== "object") return out;
  for (const k of ["gclid", "gbraid", "wbraid", "fbc", "fbp"] as const) {
    const v = customData[k];
    if (typeof v === "string" && v) out[k] = v;
  }
  return out;
}

/** Turns a paid order into a licence, exactly once.
 *
 *  Paddle retries the webhook on any non-2xx, so this must never issue a
 *  second licence — and a second charge's worth of seats — for one payment.
 *  The check-and-act used to be spread over separate statements, which two
 *  near-simultaneous deliveries could interleave; [`claimOrderAndIssue`] does
 *  the check and the creation in one transaction, so exactly one delivery
 *  wins and the loser finds the order already paid. */
async function fulfil(
  env: Env,
  ref: string,
  opts: { provider: string; cycle: Cycle; email: string | null; providerRef: string | null },
): Promise<boolean> {
  // Read only for the address fallback: the webhook's own email wins, and
  // the one typed at checkout is what delivers the key when the webhook has
  // none. Whether the order is paid is decided inside the claim.
  const order = await orderByRef(env, ref);
  if (!order) {
    console.error(`fulfilment for an unknown order ${ref}`);
    return false;
  }

  const outcome = await claimOrderAndIssue(env, ref, {
    provider: opts.provider,
    cycle: opts.cycle,
    email: opts.email ?? order.email,
    providerRef: opts.providerRef,
  });
  if (!outcome.issued || !outcome.id || !outcome.key) {
    // Already paid: either an earlier delivery finished, or one is finishing
    // right now. Either way this delivery must not answer with an error, or
    // Paddle would retry a fulfilment that needs no retrying.
    return false;
  }
  console.log(`issued licence ${outcome.id} for order ${ref} via ${opts.provider}`);
  await deliverKey(env, opts.email ?? order.email, outcome.key, opts.cycle, outcome.expiresAt!);
  return true;
}

async function paddleWebhook(env: Env, request: Request): Promise<Response> {
  // The address first, then the MAC. Neither stands in for the other: this
  // drops anything that did not arrive from Paddle's own network, and the
  // signature below is what proves the payload is theirs. A refusal here is
  // not 2xx, so Paddle retries -- which is the right outcome if the address
  // check is ever wrong.
  if (!(await fromPaddle(request))) {
    await logWebhook(env, { outcome: "refused", detail: request.headers.get("CF-Connecting-IP") ?? "" });
    return refuse("Forbidden.", 403);
  }

  const raw = await request.text();
  const event = await verifyWebhook(env, raw, request.headers.get("Paddle-Signature"));
  if (!event) {
    await logWebhook(env, { outcome: "bad signature" });
    return refuse("Bad signature.", 401);
  }

  const entity = event.data?.id ? String(event.data.id) : null;
  const logged = { eventType: event.eventType, eventId: event.eventId, entity };
  let response: Response;
  try {
    response = await handlePaddleEvent(env, event);
  } catch (err) {
    await logWebhook(env, { ...logged, outcome: "error", detail: String(err) });
    throw err;
  }
  // Every handler answers `{ ok, ignored? }`; the reason it gives for
  // ignoring is exactly what the panel should say.
  const body: any = await response.clone().json().catch(() => ({}));
  await logWebhook(env, {
    ...logged,
    outcome: body?.ignored !== undefined ? "ignored" : "handled",
    detail: body?.ignored !== undefined ? String(body.ignored) : null,
  });
  return response;
}

/** Writes one delivery to `webhook_events` for the admin panel. Never fails
 *  the delivery: the log is a convenience, and a webhook answered with an
 *  error because its log line could not be written would be retried for a
 *  payment that was already handled. */
async function logWebhook(
  env: Env,
  entry: {
    outcome: string;
    eventType?: string;
    eventId?: string | null;
    entity?: string | null;
    detail?: string | null;
  },
): Promise<void> {
  try {
    const t = now();
    await env.DB.batch([
      env.DB.prepare(
        `INSERT INTO webhook_events (at, event_type, event_id, entity_id, outcome, detail)
         VALUES (?, ?, ?, ?, ?, ?)`,
      ).bind(
        t,
        entry.eventType ?? null,
        entry.eventId ?? null,
        entry.entity ?? null,
        entry.outcome,
        entry.detail ? entry.detail.slice(0, 200) : null,
      ),
      env.DB.prepare("DELETE FROM webhook_events WHERE at < ?").bind(t - 90 * 86_400),
    ]);
  } catch (err) {
    console.error("could not log a webhook delivery", err);
  }
}

async function handlePaddleEvent(env: Env, event: Verified): Promise<Response> {
  const data = event.data ?? {};
  // Paddle's own idea of when this happened. Deliveries are at-least-once and
  // unordered, so every mirror write is guarded by it: an old payload arriving
  // late must not overwrite a newer status.
  const eventAt = occurredAt(event);

  switch (event.eventType) {
    case "transaction.completed": {
      const transactionId = data.id ? String(data.id) : null;
      if (transactionId && !(await claimTransaction(env, transactionId))) {
        return json({ ok: true, ignored: "already handled" });
      }
      try {
        return await transactionCompleted(env, data, eventAt);
      } catch (err) {
        if (transactionId) await releaseTransaction(env, transactionId);
        throw err;
      }
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

    case "adjustment.created":
    case "adjustment.updated": {
      // Refunds and chargebacks arrive as adjustments, and only the money
      // actually going back cuts the term short. A refund is created as
      // `pending_approval` and may yet be rejected, so it counts once it is
      // approved — which is why the update is listened to as well. A partial
      // refund is a goodwill credit, not the purchase undone, and leaves the
      // licence alone -- unless every item on it is refunded in `full`, which
      // is what the Paddle dashboard sends for a whole refund: `type` is
      // "partial" because it was made per item, but the purchase is undone.
      // A chargeback is the buyer taking the money back without asking, and
      // ends it the same as a refund.
      //
      // Every adjustment is recorded for the panel's refund tracker first,
      // whatever happens to the licence below.
      await mirrorAdjustment(env, data, eventAt);
      const action = String(data.action ?? "");
      const status = String(data.status ?? "");
      const ends = (action === "refund" && refundsWholePurchase(data)) || action === "chargeback";
      if (!ends) return json({ ok: true, ignored: data.type === "partial" ? "partial refund" : action });
      if (status !== "approved") return json({ ok: true, ignored: `${action} ${status}` });
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

/** Whether a refund gives back the whole purchase. True for `type: "full"`,
 *  and for a "partial" refund whose every item is refunded in full -- the
 *  shape the Paddle dashboard uses for a whole refund. A transaction here
 *  carries the one plan item, so all items full is the whole purchase. */
export function refundsWholePurchase(data: any): boolean {
  if (data?.type !== "partial") return true;
  const items: any[] = Array.isArray(data?.items) ? data.items : [];
  return items.length > 0 && items.every((i) => i?.type === "full");
}

/** A payment: the first one sells a licence, every later one renews it. */
async function transactionCompleted(env: Env, data: any, eventAt: number): Promise<Response> {
  const priced = cycleFromItems(env, data.items ?? []);
  const cycle = priced ?? "monthly";
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
      // A price no longer in the config (one replaced by a new price) still
      // renews the plan the licence was bought on, not a month of it.
      const renewed = priced ?? (isCycle(existing.cycle) ? existing.cycle : "monthly");
      const until = await extendTerm(env, existing, renewed);
      console.log(`extended licence ${existing.id} to ${until}`);
      await noteCountry(env, subscriptionId, data);
      return json({ ok: true });
    }
  }

  if (!ref) {
    console.error("Paddle transaction with no order ref and no known subscription");
    return json({ ok: true, ignored: "no ref" });
  }
  const newlyPaid = await fulfil(env, ref, {
    provider: "paddle",
    cycle,
    email,
    providerRef: subscriptionId,
  });
  if (subscriptionId) await noteCountry(env, subscriptionId, data);
  // First payment only: renewals return above and a retried webhook finds
  // the order already paid, so the conversion is pushed exactly once.
  if (newlyPaid) {
    await pushConversions(env, {
      clickIds: readClickIds(data?.custom_data),
      cycle,
      orderRef: ref,
      email,
      // occurredAt is in seconds; ads want ms, and a missing stamp (0)
      // falls back to now rather than 1970.
      eventTimeMs: eventAt ? eventAt * 1000 : Date.now(),
    });
  }
  return json({ ok: true });
}

/** Records which country a licence was bought from, for the admin panel. Only
 *  fills an empty column, so a renewal also back-fills a licence sold before
 *  the column existed. Never fails the webhook: the payment is already done. */
async function noteCountry(env: Env, subscriptionId: string, data: any): Promise<void> {
  try {
    const country = await customerCountry(env, data);
    if (!country) return;
    await env.DB.prepare(
      "UPDATE licences SET country = ? WHERE provider = 'paddle' AND provider_ref = ? AND country IS NULL",
    )
      .bind(country, subscriptionId)
      .run();
  } catch (err) {
    console.error("could not record the licence's country", err);
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
      // Only Paddle subscriptions recur, so only they can be cancelled.
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
      // Handed to Retain on the page. Null until a webhook has named the
      // customer, which is the same moment the portal button appears.
      customerId: subscription?.customer_id ?? null,
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
    undefined,
    // Only when Paddle is actually configured: `paddleEnvOrThrow` refuses to
    // guess, and the account page must not fail to render over a Retain
    // nicety.
    paddleConfigured(env)
      ? { clientToken: env.PADDLE_CLIENT_TOKEN, env: paddleEnvOrThrow(env) }
      : undefined,
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
  await recordCancelReason(env, licence, form.get("reason"), form.get("note"));

  // Paddle will also send `subscription.canceled`; recording it now means the
  // page the user is about to see tells the truth without waiting for it.
  await endLicence(env, licence, "cancelled");
  const updated = (await licenceById(env, licence.id)) ?? licence;
  return accountView(env, ctx, updated, key, s.cancelled(updated.renews_at ?? s.endOfPaidPeriod));
}

/** Keeps the answer to "why are you leaving?", when there is one. Only after
 *  Paddle has accepted the cancellation, so a failed one leaves no reason
 *  behind, and never in the way of it: the subscription is already cancelled
 *  by the time this could fail. */
async function recordCancelReason(
  env: Env,
  licence: Licence,
  rawReason: unknown,
  rawNote: unknown,
): Promise<void> {
  const reason = typeof rawReason === "string" ? rawReason : "";
  const note = typeof rawNote === "string" ? rawNote.trim().slice(0, 500) : "";
  if (!isCancelReason(reason) && !note) return;
  try {
    await env.DB.prepare(
      `INSERT INTO cancel_reasons (at, licence_id, cycle, reason, note) VALUES (?, ?, ?, ?, ?)`,
    )
      .bind(now(), licence.id, licence.cycle, isCancelReason(reason) ? reason : "other", note || null)
      .run();
  } catch (err) {
    console.error("could not record a cancel reason", err);
  }
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
  // The weekly report and the daily alerts and forgetting run off cron
  // triggers (see wrangler.toml). waitUntil keeps the isolate alive until they
  // finish; the sends are best-effort and never throw, so a mail failure
  // cannot fail the scheduled run.
  async scheduled(controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(
      controller.cron === "0 6 * * *"
        ? Promise.all([sendAlerts(env), forgetOld(env)])
        : sendWeeklyReport(env),
    );
  },

  async fetch(request: Request, env: Env): Promise<Response> {
    return withSecurityHeaders(await route(request, env));
  },
} satisfies ExportedHandler<Env>;

/** Headers every response carries.
 *
 *  The account page shows a licence key and has a cancel button, so it must
 *  not be framed by another site (clickjacking) and must not leak its address
 *  in a Referer. HSTS because this host is only ever served over HTTPS; a
 *  first visit over plain HTTP is the one an attacker on the network gets to
 *  rewrite. Paddle's overlay is an iframe *inside* our page, which
 *  `frame-ancestors` does not restrict. */
function withSecurityHeaders(response: Response): Response {
  const out = new Response(response.body, response);
  out.headers.set("Strict-Transport-Security", "max-age=31536000; includeSubDomains");
  out.headers.set("X-Content-Type-Options", "nosniff");
  out.headers.set("Referrer-Policy", "strict-origin-when-cross-origin");
  if ((out.headers.get("content-type") ?? "").includes("text/html")) {
    out.headers.set("X-Frame-Options", "DENY");
    out.headers.set("Content-Security-Policy", "frame-ancestors 'none'");
  }
  return out;
}

async function route(request: Request, env: Env): Promise<Response> {
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
      if (pathname === "/v1/config") {
        const os = url.searchParams.get("os") ?? "";
        const channel = url.searchParams.get("channel") ?? "";
        if (!["linux", "windows", "macos"].includes(os) || !["store", "download"].includes(channel)) return refuse("Invalid configuration scope.");
        const app = url.searchParams.get("app") ?? "";
        return json(await remoteSettings(env, os, channel, /^\d{1,9}\.\d{1,9}\.\d{1,9}$/.test(app) ? app : null));
      }
      // The checkout domain is reviewed as a site in its own right: Paddle
      // fetches it and looks for a real front page and the three policies.
      // They are written once, on the marketing site, so this points at
      // them rather than answering 404 and failing the review.
      if (pathname === "/") return redirect(SITE + "/");
      if (pathname === "/terms" || pathname === "/privacy" || pathname === "/refunds") {
        return redirect(SITE + pathname);
      }
      // Only the buy page is worth finding in a search. The welcome page is
      // one buyer's order, the account page is a form, and the rest is API.
      if (pathname === "/robots.txt") {
        return new Response(
          "User-agent: *\nAllow: /buy\nDisallow: /\n\nSitemap: " + SITE + "/sitemap.xml\n",
          { headers: { "content-type": "text/plain; charset=utf-8" } },
        );
      }
      if (pathname === "/buy") return buy(env, request, url);
      if (pathname === "/account") {
        return accountPage(pageContext(request, url), null, supportEmail(env));
      }
      // /welcome is where Paddle returns the buyer. It polls for the licence
      // key using the ref in the query string.
      if (pathname === "/welcome") {
        const ctx = pageContext(request, url);
        const ref = url.searchParams.get("ref") ?? "";
        // The page writes the ref back into itself, so only the shape
        // `newOrderRef` mints is let through -- anything else is not an order.
        if (!/^[0-9a-f]{32}$/.test(ref)) return redirect(withLang("/buy", ctx.lang));
        const order = await orderByRef(env, ref);
        const conversion =
          env.GOOGLE_ADS_TAG_ID && env.GOOGLE_ADS_PURCHASE_LABEL && order && isCycle(order.cycle)
            ? {
                tagId: env.GOOGLE_ADS_TAG_ID,
                sendTo: `${env.GOOGLE_ADS_TAG_ID}/${env.GOOGLE_ADS_PURCHASE_LABEL}`,
                value: USD_AMOUNT[order.cycle],
              }
            : null;
        const sale =
          order && isCycle(order.cycle) ? { cycle: order.cycle, value: USD_AMOUNT[order.cycle] } : null;
        return donePage(ctx, ref, supportEmail(env), conversion, sale);
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
      case "/webhooks/paddle":
        return await paddleWebhook(env, request);
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
      case "/v1/ping":
        return await ping(env, body, request);
      case "/v1/alive":
        return await alive(env, body);
      case "/v1/deepl":
        return await deeplRelay(env, body);
      case "/v1/word":
        return await wordRelay(env, body);
      case "/v1/download":
        return await downloadClick(env, body, request);
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
}
