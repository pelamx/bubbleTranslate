// The operator's panel: who is subscribed, and the four things support
// actually has to do about it.
//
// It exists because the alternative was a README full of `wrangler d1 execute`
// incantations, and a support task you can only do by hand-writing SQL against
// production is a support task that eventually gets done wrong. Everything
// here is something that came up as a real question: how many subscribers are
// there, who is this person who emailed, extend them, free their devices,
// give them a working key again.
//
// It is deliberately small. There is no charging, no refunding and no
// cancelling of a subscription from here — that lives with Paddle, which owns
// the money, and doing it in two places is how the two disagree.

import { type Cycle, type Env, isCycle } from "./env";
import {
  DEFAULT_SEATS,
  MANUAL_PROVIDER,
  endLicence,
  issueLicence,
  licenceById,
  rotateKey,
} from "./licences";
import { reportPage } from "./report";
import { grantsAccess, subscriptionById } from "./mirror";
import { constantTimeEqual, now, sha256Hex } from "./tokens";
import { DAY, date } from "./admin/shared";
import { dashboard } from "./admin/render";

// -- getting in --------------------------------------------------------------

/** Failed attempts and when the next one may be tried, per isolate.
 *
 *  A shared counter across isolates would need Durable Objects, which is not
 *  a price worth paying for a panel this size — so this is a brake rather
 *  than a vault. The vault is the password itself; what the brake buys is
 *  that a guesser hammering the route from one connection pays a growing
 *  delay instead of an unthrottled oracle. */
const authBackoff = { failures: 0, notBefore: 0 };

/** HTTP Basic, checked against `ADMIN_PASSWORD`.
 *
 *  Both sides are hashed before they are compared, so the comparison is over
 *  two fixed-length strings and leaks neither the password's length nor where
 *  a guess first went wrong. */
async function authorised(env: Env, request: Request): Promise<boolean> {
  const expected = env.ADMIN_PASSWORD;
  if (!expected) return false;

  // The backoff is checked before the password is hashed: an attempt during
  // a lockout must not cost a hash, and must not be answered faster than the
  // lockout says. Every failure feeds it — an absent header included, since
  // a guesser probes both ways.
  if (Date.now() < authBackoff.notBefore) return false;

  const header = request.headers.get("authorization") ?? "";
  if (!header.startsWith("Basic ")) return false;

  let decoded: string;
  try {
    decoded = atob(header.slice(6));
  } catch {
    return false;
  }
  // Basic is `user:password`; the user half is ignored, so any name works.
  const supplied = decoded.slice(decoded.indexOf(":") + 1);
  const ok = constantTimeEqual(await sha256Hex(supplied), await sha256Hex(expected));

  if (ok) {
    authBackoff.failures = 0;
    authBackoff.notBefore = 0;
    return true;
  }
  // Each failure buys the next attempt a longer wait: 1s, 3s, 7s … capped at
  // an hour. Per isolate, so a patient attacker with several isolates in play
  // still pays this table once each — but the credential space is large
  // enough that the brake only has to make guessing slower than support.
  authBackoff.failures += 1;
  const delaySeconds = Math.min(2 ** authBackoff.failures - 1, 3600);
  authBackoff.notBefore = Date.now() + delaySeconds * 1000;
  return false;
}

const challenge = () =>
  new Response("Unauthorized", {
    status: 401,
    headers: { "www-authenticate": 'Basic realm="bubbleTranslate admin"' },
  });

/** A browser that has been given Basic credentials once will attach them to
 *  *any* request it is talked into making, including a form on someone else's
 *  page. So a state-changing request has to prove it came from here. */
function sameOrigin(request: Request): boolean {
  const origin = request.headers.get("origin");
  if (!origin) return true; // no Origin at all is not a cross-site form post
  try {
    return new URL(origin).origin === new URL(request.url).origin;
  } catch {
    return false;
  }
}

/** Records what the operator just did, for the panel's history. Written
 *  after the change itself, so a failed change leaves no line claiming it
 *  happened; and never allowed to fail the change it describes. */
async function audit(env: Env, action: string, licenceId: string | null, detail: string) {
  try {
    await env.DB.prepare("INSERT INTO admin_log (at, action, licence_id, detail) VALUES (?, ?, ?, ?)")
      .bind(now(), action, licenceId, detail)
      .run();
  } catch (err) {
    console.error("could not write the admin log", err);
  }
}

/** Marks an install as the operator's own, or takes the mark off. */
async function markMine(env: Env, request: Request): Promise<Response> {
  if (!sameOrigin(request)) return new Response("Cross-site request refused.", { status: 403 });
  const form = await request.formData();
  const install = String(form.get("install") ?? "").trim();
  if (install) {
    if (form.get("mine") === "1") {
      await env.DB.prepare("INSERT OR IGNORE INTO ignored_installs (install, created_at) VALUES (?, ?)")
        .bind(install, now())
        .run();
    } else {
      await env.DB.prepare("DELETE FROM ignored_installs WHERE install = ?").bind(install).run();
    }
  }
  return new Response(null, { status: 303, headers: { location: "/admin#users" } });
}

/** Marks a licence as the operator's own -- a test purchase -- or takes the
 *  mark off. Only the counts change: the licence itself is left exactly as it
 *  was, and still works. */
async function markMineLicence(env: Env, request: Request): Promise<Response> {
  if (!sameOrigin(request)) return new Response("Cross-site request refused.", { status: 403 });
  const form = await request.formData();
  const id = String(form.get("id") ?? "").trim();
  if (id) {
    if (form.get("mine") === "1") {
      await env.DB.prepare("INSERT OR IGNORE INTO ignored_licences (licence_id, created_at) VALUES (?, ?)")
        .bind(id, now())
        .run();
    } else {
      await env.DB.prepare("DELETE FROM ignored_licences WHERE licence_id = ?").bind(id).run();
    }
    await audit(env, "mine", id, form.get("mine") === "1" ? "marked as mine" : "unmarked as mine");
  }
  return new Response(null, { status: 303, headers: { location: "/admin#licences" } });
}

// -- acting on it ------------------------------------------------------------

/** Creates a licence nobody paid for.
 *
 *  Deliberately not a variant of `act`: everything there starts by finding an
 *  existing licence, and this one has none to find.
 *
 *  `provider` is `manual` rather than `paddle`, which is what keeps it out of
 *  the processor's way. A Paddle webhook finds its row by `provider_ref`, so a
 *  row with none is never moved by one; and the cancel and portal routes both
 *  check `provider === "paddle"` before offering anything, so the account page
 *  shows this licence without a cancel button it could not honour. */
async function issue(env: Env, request: Request): Promise<Response> {
  if (!sameOrigin(request)) return new Response("Cross-site request refused.", { status: 403 });

  const form = await request.formData();
  const cycle: Cycle = isCycle(form.get("cycle")) ? (form.get("cycle") as Cycle) : "yearly";

  const email = String(form.get("email") ?? "").trim() || null;

  const rawSeats = String(form.get("seats") ?? "").trim();
  const seats = rawSeats ? Number(rawSeats) : DEFAULT_SEATS;
  if (!Number.isInteger(seats) || seats < 1 || seats > 20) {
    return dashboard(env, "", { error: "Devices must be between 1 and 20." });
  }

  // Blank means "however long that cycle is", which is the common case.
  const rawDays = String(form.get("days") ?? "").trim();
  const days = rawDays ? Number(rawDays) : null;
  if (days !== null && (!Number.isInteger(days) || days < 1 || days > 400)) {
    return dashboard(env, "", { error: "Days must be between 1 and 400." });
  }

  const { id, key, expiresAt } = await issueLicence(env, {
    provider: MANUAL_PROVIDER,
    cycle,
    email,
    seats,
    termSeconds: days === null ? undefined : days * DAY,
  });

  await audit(env, "issue", id, `${cycle}, ${seats} devices, ends ${date(expiresAt)}${email ? `, ${email}` : ""}`);
  return dashboard(env, id, {
    key,
    message: `Issued ${id} — ${cycle}, ${seats} device${seats === 1 ? "" : "s"}, ends ${date(expiresAt)}.`,
  });
}

/** Every mutation ends by re-rendering the dashboard filtered to the licence
 *  that was touched, so the operator sees the result rather than a redirect to
 *  a list they then have to search again. */
async function act(env: Env, request: Request, action: string): Promise<Response> {
  if (!sameOrigin(request)) return new Response("Cross-site request refused.", { status: 403 });

  const form = await request.formData();
  const id = String(form.get("id") ?? "").trim();
  const licence = id ? await licenceById(env, id) : null;
  if (!licence) return dashboard(env, id, { error: "That licence no longer exists." });

  switch (action) {
    case "extend": {
      const days = Number(form.get("days"));
      if (!Number.isInteger(days) || days < 1 || days > 400) {
        return dashboard(env, id, { error: "Extension must be between 1 and 400 days." });
      }
      // From whichever is later, so extending a lapsed licence gives the days
      // rather than spending them on the time it spent expired.
      const from = Math.max(licence.expires_at, now());
      const expiresAt = from + days * DAY;
      await env.DB.prepare("UPDATE licences SET expires_at = ?, renews_at = ? WHERE id = ?")
        .bind(expiresAt, date(expiresAt), licence.id)
        .run();
      await audit(env, "extend", licence.id, `+${days} days: ${date(licence.expires_at)} → ${date(expiresAt)}`);
      return dashboard(env, id, {
        message: `Extended by ${days} days — now ends ${date(expiresAt)}.`,
      });
    }

    case "seats": {
      const { meta } = await env.DB.prepare("DELETE FROM seats WHERE licence_id = ?")
        .bind(licence.id)
        .run();
      await audit(env, "free seats", licence.id, `freed ${meta?.changes ?? 0} devices`);
      return dashboard(env, id, {
        message: `Freed ${meta?.changes ?? 0} device slots. The customer can activate again.`,
      });
    }

    case "rotate": {
      const key = await rotateKey(env, licence);
      await audit(env, "new key", licence.id, "old key disabled");
      return dashboard(env, id, {
        key,
        message:
          "New key issued. Machines already activated keep working — only the old key is dead.",
      });
    }

    case "update": {
      const email = String(form.get("email") ?? "").trim() || null;

      const cycle = String(form.get("cycle") ?? "");
      if (!isCycle(cycle)) return dashboard(env, id, { error: "Cycle must be monthly or yearly." });

      const status = String(form.get("status") ?? "");
      if (!["active", "cancelled", "refunded"].includes(status)) {
        return dashboard(env, id, { error: "Status must be active, cancelled or refunded." });
      }

      const seatLimit = Number(form.get("seat_limit"));
      if (!Number.isInteger(seatLimit) || seatLimit < 1 || seatLimit > 20) {
        return dashboard(env, id, { error: "Devices must be between 1 and 20." });
      }

      // A date, not a duration: this is the field you reach for when the term
      // has to land on a day someone was promised, rather than a month from
      // whenever the button was pressed.
      const day = String(form.get("expires_at") ?? "").trim();
      const expiresAt = Math.floor(Date.parse(`${day}T00:00:00Z`) / 1000);
      if (!/^\d{4}-\d{2}-\d{2}$/.test(day) || !Number.isFinite(expiresAt)) {
        return dashboard(env, id, { error: "Ends must be a date." });
      }

      // Blank is unlimited, matching the NULL the column already means.
      const rawLimit = String(form.get("translation_limit") ?? "").trim();
      const limit = rawLimit ? Number(rawLimit) : null;
      if (limit !== null && (!Number.isInteger(limit) || limit < 0)) {
        return dashboard(env, id, { error: "Translations must be a whole number, or blank." });
      }

      await env.DB.prepare(
        `UPDATE licences
            SET email = ?, cycle = ?, status = ?, seat_limit = ?,
                expires_at = ?, renews_at = ?, translation_limit = ?
          WHERE id = ?`,
      )
        .bind(email, cycle, status, seatLimit, expiresAt, date(expiresAt), limit, licence.id)
        .run();

      // Only what actually changed, old → new, so the history reads as a
      // list of decisions rather than a dump of the form.
      const changes = [
        ["email", licence.email ?? "—", email ?? "—"],
        ["cycle", licence.cycle, cycle],
        ["status", licence.status, status],
        ["devices", String(licence.seat_limit), String(seatLimit)],
        ["ends", date(licence.expires_at), date(expiresAt)],
        ["translations", String(licence.translation_limit ?? "unlimited"), String(limit ?? "unlimited")],
      ]
        .filter(([, from, to]) => from !== to)
        .map(([field, from, to]) => `${field} ${from} → ${to}`);
      await audit(env, "edit", licence.id, changes.join(", ") || "saved without changes");

      return dashboard(env, id, { message: `Saved. Ends ${date(expiresAt)}.` });
    }

    case "delete": {
      // The one irreversible thing in here, and the one place the panel can
      // quietly break the processor's side.
      //
      // A renewal arrives as `transaction.completed` carrying the subscription
      // id and no order ref. The handler finds the licence by `provider_ref`,
      // and when there is no row to find it logs `no ref` and answers 2xx --
      // so Paddle goes on charging a card every month for a licence that will
      // never exist again, and nothing anywhere says so. Cancelling in Paddle
      // first is what stops the money; then there is nothing left to break.
      if (licence.provider === "paddle" && licence.provider_ref) {
        const subscription = await subscriptionById(env, licence.provider_ref);
        if (subscription && grantsAccess(subscription)) {
          return dashboard(env, id, {
            error:
              `${licence.id} still has a live Paddle subscription (${licence.provider_ref}). ` +
              "Cancel it in Paddle first, or the card keeps being charged for a licence " +
              "this would delete. Refund or Cancel here if you only meant to end the term.",
          });
        }
      }

      const batch = await env.DB.batch([
        env.DB.prepare("DELETE FROM seats WHERE licence_id = ?").bind(licence.id),
        env.DB.prepare("DELETE FROM licences WHERE id = ?").bind(licence.id),
      ]);
      const freed = batch[0]?.meta?.changes ?? 0;
      await audit(
        env,
        "delete",
        licence.id,
        `${licence.provider} ${licence.cycle}${licence.email ? `, ${licence.email}` : ""}, ${freed} devices`,
      );

      // Back to the unfiltered list: searching for the id just deleted would
      // render "Nothing matched", which reads like a failure.
      return dashboard(env, "", {
        message: `Deleted ${licence.id} and ${freed} device slot${freed === 1 ? "" : "s"}. Its key is dead.`,
      });
    }

    case "end": {
      const status = String(form.get("status") ?? "");
      if (status !== "refunded" && status !== "cancelled") {
        return dashboard(env, id, { error: "Unknown status." });
      }
      await endLicence(env, licence, status);
      await audit(env, status === "refunded" ? (licence.provider === "paddle" ? "refund" : "revoke") : "cancel", licence.id, `was ${licence.status}, paid to ${date(licence.expires_at)}`);
      return dashboard(env, id, {
        message:
          status === "refunded"
            ? "Marked refunded — Pro ends now. Refund the money in the processor separately."
            : "Marked cancelled — Pro runs to the end of the paid term.",
      });
    }

    default:
      return new Response("Not found.", { status: 404 });
  }
}

// -- taking it away ----------------------------------------------------------

/** One CSV cell. Quoted always, and a value that a spreadsheet would read as
 *  a formula — an email address someone chose to start with `=` — is
 *  prefixed with a quote so it stays text. */
function cell(value: unknown): string {
  let text = value === null || value === undefined ? "" : String(value);
  if (/^[=+\-@\t\r]/.test(text)) text = `'${text}`;
  return `"${text.replace(/"/g, '""')}"`;
}

const csvRows = (header: string[], rows: Record<string, unknown>[]) =>
  [header, ...rows.map((r) => header.map((h) => r[h]))]
    .map((line) => line.map(cell).join(","))
    .join("\r\n") + "\r\n";

/** Every licence, or every Paddle customer, as a spreadsheet — for the
 *  accountant, or a copy kept somewhere other than this database. Keys are
 *  not in it: the database only holds their hashes, and those are left out
 *  too. Dates are UTC days. */
async function exportCsv(env: Env, what: string): Promise<Response> {
  let body: string;
  if (what === "licences") {
    const { results } = await env.DB.prepare(
      `SELECT l.id, l.email, l.provider, l.provider_ref, l.plan, l.cycle, l.status, l.seat_limit,
              (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id) AS devices,
              l.translation_limit, l.created_at, l.expires_at,
              EXISTS (SELECT 1 FROM ignored_licences i WHERE i.licence_id = l.id) AS mine
         FROM licences l ORDER BY l.created_at`,
    ).all<Record<string, unknown>>();
    const rows = (results ?? []).map((r) => ({
      ...r,
      created: date(Number(r.created_at)),
      ends: date(Number(r.expires_at)),
      mine: r.mine ? "yes" : "",
    }));
    body = csvRows(
      ["id", "email", "provider", "provider_ref", "plan", "cycle", "status", "seat_limit", "devices",
        "translation_limit", "created", "ends", "mine"],
      rows,
    );
  } else if (what === "customers") {
    const { results } = await env.DB.prepare(
      `SELECT c.customer_id, c.email, c.status AS customer_status, s.subscription_id,
              s.status AS subscription_status, s.price_id, s.next_billed_at,
              s.scheduled_change_action, c.created_at
         FROM paddle_customers c
         LEFT JOIN paddle_subscriptions s ON s.customer_id = c.customer_id
        ORDER BY c.created_at`,
    ).all<Record<string, unknown>>();
    const rows = (results ?? []).map((r) => ({ ...r, first_seen: date(Number(r.created_at)) }));
    body = csvRows(
      ["customer_id", "email", "customer_status", "subscription_id", "subscription_status", "price_id",
        "next_billed_at", "scheduled_change_action", "first_seen"],
      rows,
    );
  } else {
    return new Response("Not found.", { status: 404 });
  }
  return new Response(body, {
    headers: {
      "content-type": "text/csv; charset=utf-8",
      "content-disposition": `attachment; filename="bubbletranslate-${what}-${date(now())}.csv"`,
      "cache-control": "no-store",
    },
  });
}

// -- the entry point ---------------------------------------------------------

/** Handles anything under `/admin`, or returns null so the router carries on.
 *
 *  Returns 404 rather than 401 when no password is configured: an unset
 *  `ADMIN_PASSWORD` should look like a service with no admin panel, not like
 *  one with an admin panel and a lock to pick. */
export async function handleAdmin(
  env: Env,
  request: Request,
  url: URL,
): Promise<Response | null> {
  const { pathname } = url;
  if (pathname !== "/admin" && !pathname.startsWith("/admin/")) return null;

  if (!env.ADMIN_PASSWORD) return new Response("Not found.", { status: 404 });
  if (!(await authorised(env, request))) return challenge();

  if (request.method === "GET" && pathname === "/admin/report") {
    return reportPage(env);
  }
  if (request.method === "GET" && pathname === "/admin/export.csv") {
    return exportCsv(env, url.searchParams.get("what") ?? "");
  }
  if (request.method === "GET" && pathname === "/admin") {
    return dashboard(env, url.searchParams.get("q") ?? "");
  }
  if (request.method === "POST") {
    const action = pathname.slice("/admin/".length);
    if (action === "issue") return issue(env, request);
    if (action === "mine") return markMine(env, request);
    if (action === "mine-licence") return markMineLicence(env, request);
    if (["extend", "seats", "rotate", "end", "update", "delete"].includes(action)) {
      return act(env, request, action);
    }
  }
  return new Response("Not found.", { status: 404 });
}
