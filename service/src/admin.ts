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

import { type Cycle, type Env, isCycle, supportEmail } from "./env";
import {
  DEFAULT_SEATS,
  type Licence,
  MANUAL_PROVIDER,
  endLicence,
  isLive,
  issueLicence,
  licenceById,
  rotateKey,
} from "./licences";
import { grantsAccess, subscriptionById } from "./mirror";
import { escapeHtml, page } from "./pages";
import { constantTimeEqual, now, sha256Hex } from "./tokens";

/** How many rows a listing shows. Enough to scan, few enough to render. */
const PAGE_SIZE = 40;

const DAY = 86_400;

// -- getting in --------------------------------------------------------------

/** HTTP Basic, checked against `ADMIN_PASSWORD`.
 *
 *  Both sides are hashed before they are compared, so the comparison is over
 *  two fixed-length strings and leaks neither the password's length nor where
 *  a guess first went wrong. */
async function authorised(env: Env, request: Request): Promise<boolean> {
  const expected = env.ADMIN_PASSWORD;
  if (!expected) return false;

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
  return constantTimeEqual(await sha256Hex(supplied), await sha256Hex(expected));
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

// -- reading the state -------------------------------------------------------

interface Stats {
  live: number;
  monthly: number;
  yearly: number;
  paddle: number;
  winding_down: number;
  expiring: number;
  fresh: number;
  total: number;
}

async function stats(env: Env): Promise<Stats> {
  const t = now();
  // One pass with conditional sums rather than eight queries. "Live" here is
  // the same rule `isLive` applies, spelled in SQL: not refunded, and the term
  // has not run out.
  const row = await env.DB.prepare(
    `SELECT
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?  THEN 1 ELSE 0 END) AS live,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?  AND cycle = 'monthly' THEN 1 ELSE 0 END) AS monthly,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?  AND cycle = 'yearly'  THEN 1 ELSE 0 END) AS yearly,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?  AND provider = 'paddle' THEN 1 ELSE 0 END) AS paddle,
       SUM(CASE WHEN status  = 'cancelled' AND expires_at > ? THEN 1 ELSE 0 END) AS winding_down,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?  AND expires_at < ? THEN 1 ELSE 0 END) AS expiring,
       SUM(CASE WHEN created_at > ? THEN 1 ELSE 0 END) AS fresh,
       COUNT(*) AS total
     FROM licences`,
  )
    .bind(t, t, t, t, t, t, t + 7 * DAY, t - 30 * DAY)
    .first<Stats>();

  return (
    row ?? {
      live: 0,
      monthly: 0,
      yearly: 0,
      paddle: 0,
      winding_down: 0,
      expiring: 0,
      fresh: 0,
      total: 0,
    }
  );
}

interface Row extends Licence {
  created_at: number;
  seats: number;
}

const SELECT_ROWS = `
  SELECT l.id, l.plan, l.cycle, l.translation_limit, l.status, l.seat_limit,
         l.expires_at, l.renews_at, l.email, l.provider, l.provider_ref,
         l.created_at,
         (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id) AS seats
    FROM licences l`;

/** Finds licences by whatever the operator pasted into the box.
 *
 *  Three shapes, because support arrives in three shapes: a licence key from
 *  the customer's email, a licence id from a log line, or — most often — just
 *  the address they wrote from. The key is hashed before it is looked up,
 *  since that is the only form the database holds. */
async function search(env: Env, query: string): Promise<Row[]> {
  const term = query.trim();
  if (!term) {
    const { results } = await env.DB.prepare(
      `${SELECT_ROWS} ORDER BY l.created_at DESC LIMIT ?`,
    )
      .bind(PAGE_SIZE)
      .all<Row>();
    return results ?? [];
  }

  if (/^BT-/i.test(term)) {
    const licence = await env.DB.prepare(`${SELECT_ROWS} WHERE l.key_hash = ?`)
      .bind(await sha256Hex(term.toUpperCase()))
      .first<Row>();
    return licence ? [licence] : [];
  }

  if (/^lc_/.test(term)) {
    const licence = await env.DB.prepare(`${SELECT_ROWS} WHERE l.id = ?`)
      .bind(term)
      .first<Row>();
    return licence ? [licence] : [];
  }

  const { results } = await env.DB.prepare(
    `${SELECT_ROWS} WHERE l.email LIKE ? OR l.provider_ref = ? ORDER BY l.created_at DESC LIMIT ?`,
  )
    .bind(`%${term}%`, term, PAGE_SIZE)
    .all<Row>();
  return results ?? [];
}

/** Recent checkouts that did not become licences. Worth a glance: a run of
 *  these is a broken price, a wrong credential or a processor outage, and none
 *  of those announce themselves anywhere else. */
async function recentFailures(env: Env) {
  const { results } = await env.DB.prepare(
    `SELECT ref, provider, cycle, email, failure, created_at
       FROM orders WHERE status = 'failed' AND created_at > ?
       ORDER BY created_at DESC LIMIT 10`,
  )
    .bind(now() - 7 * DAY)
    .all<{
      ref: string;
      provider: string;
      cycle: string;
      email: string | null;
      failure: string | null;
      created_at: number;
    }>();
  return results ?? [];
}

// -- rendering ---------------------------------------------------------------

const date = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

function relativeDays(unix: number): string {
  const days = Math.round((unix - now()) / DAY);
  if (days < 0) return `${-days}d ago`;
  if (days === 0) return "today";
  return `in ${days}d`;
}

function statusCell(licence: Row): string {
  if (!isLive(licence)) {
    return `<span class="err">${escapeHtml(
      licence.status === "refunded" ? "refunded" : "expired",
    )}</span>`;
  }
  if (licence.status === "cancelled") {
    return '<span class="warn">ending</span>';
  }
  return '<span class="ok">active</span>';
}

/** The long way round: every field the operator might have to correct, in one
 *  form, folded away behind a summary so the table stays a table.
 *
 *  The quick buttons on the row above are the gestures support actually makes
 *  daily. This is for the rest -- a typo'd address, a term that has to land on
 *  an exact date, a seat count agreed by mail -- and it exists so that none of
 *  those ends in a hand-written UPDATE against production. */
function editPanel(r: Row): string {
  const option = (value: string, label: string, selected: string) =>
    `<option value="${escapeHtml(value)}"${value === selected ? " selected" : ""}>${escapeHtml(label)}</option>`;

  return `<details>
    <summary>Edit ${escapeHtml(r.id)}</summary>
    <form method="post" action="/admin/update" class="row edit">
      <input type="hidden" name="id" value="${escapeHtml(r.id)}">
      <div style="flex:2 1 220px">
        <label class="field" for="e-mail-${escapeHtml(r.id)}">Email</label>
        <input id="e-mail-${escapeHtml(r.id)}" type="email" name="email"
               value="${escapeHtml(r.email ?? "")}" spellcheck="false">
      </div>
      <div style="flex:0 1 130px">
        <label class="field" for="e-cyc-${escapeHtml(r.id)}">Cycle</label>
        <select id="e-cyc-${escapeHtml(r.id)}" name="cycle">
          ${option("monthly", "Monthly", r.cycle)}${option("yearly", "Yearly", r.cycle)}
        </select>
      </div>
      <div style="flex:0 1 150px">
        <label class="field" for="e-exp-${escapeHtml(r.id)}">Ends</label>
        <input id="e-exp-${escapeHtml(r.id)}" type="date" name="expires_at"
               value="${escapeHtml(date(r.expires_at))}">
      </div>
      <div style="flex:0 1 110px">
        <label class="field" for="e-seat-${escapeHtml(r.id)}">Devices</label>
        <input id="e-seat-${escapeHtml(r.id)}" type="number" name="seat_limit"
               min="1" max="20" value="${r.seat_limit}">
      </div>
      <div style="flex:0 1 140px">
        <label class="field" for="e-st-${escapeHtml(r.id)}">Status</label>
        <select id="e-st-${escapeHtml(r.id)}" name="status">
          ${option("active", "Active", r.status)}${option("cancelled", "Cancelled", r.status)}${option("refunded", "Refunded", r.status)}
        </select>
      </div>
      <div style="flex:0 1 130px">
        <label class="field" for="e-lim-${escapeHtml(r.id)}">Translations</label>
        <input id="e-lim-${escapeHtml(r.id)}" type="number" name="translation_limit" min="0"
               value="${r.translation_limit ?? ""}">
      </div>
      <button type="submit">Save</button>
    </form>
    <form method="post" action="/admin/delete" class="row">
      <input type="hidden" name="id" value="${escapeHtml(r.id)}">
      <button class="danger" type="submit"
        onclick="return confirm('Delete ${escapeHtml(r.id)} and its devices for good? The key stops working and nothing here can bring it back.')">Delete this licence</button>
    </form>
    <p class="muted">
      Blank translations means unlimited, which is what a paid licence is.
      Created ${escapeHtml(date(r.created_at))} · provider <code>${escapeHtml(r.provider)}</code>${
        r.provider_ref ? ` · ref <code>${escapeHtml(r.provider_ref)}</code>` : ""
      }
    </p>
  </details>`;
}

function rowsTable(rows: Row[]): string {
  if (!rows.length) return '<p class="muted">Nothing matched.</p>';
  const body = rows
    .map(
      (r) => `
      <tr>
        <td><code>${escapeHtml(r.id)}</code></td>
        <td>${escapeHtml(r.email ?? "—")}</td>
        <td>${escapeHtml(r.cycle)}<br><span class="muted">${escapeHtml(r.provider)}</span></td>
        <td>${statusCell(r)}</td>
        <td>${escapeHtml(date(r.expires_at))}<br><span class="muted">${escapeHtml(
          relativeDays(r.expires_at),
        )}</span></td>
        <td class="num">${r.seats}/${r.seat_limit}</td>
        <td>
          <form class="inline" method="post" action="/admin/extend">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <input type="hidden" name="days" value="30">
            <button class="quiet" type="submit">+30d</button>
          </form>
          <form class="inline" method="post" action="/admin/seats">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <button class="quiet" type="submit"
              onclick="return confirm('Free all devices on ${escapeHtml(r.id)}?')">Free seats</button>
          </form>
          <form class="inline" method="post" action="/admin/rotate">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <button class="quiet" type="submit"
              onclick="return confirm('Issue a new key? The old one stops working immediately.')">New key</button>
          </form>
          ${
            isLive(r) && r.status !== "cancelled"
              ? `<form class="inline" method="post" action="/admin/end">
                   <input type="hidden" name="id" value="${escapeHtml(r.id)}">
                   <input type="hidden" name="status" value="refunded">
                   <button class="quiet" type="submit"
                     onclick="return confirm('Mark refunded? This ends Pro immediately.')">Refund</button>
                 </form>`
              : ""
          }
        </td>
      </tr>
      <tr class="editrow"><td colspan="7">${editPanel(r)}</td></tr>`,
    )
    .join("");

  return `<div class="scroll"><table>
    <tr><th>Licence</th><th>Email</th><th>Plan</th><th>Status</th><th>Ends</th>
        <th class="num">Devices</th><th>Actions</th></tr>
    ${body}
  </table></div>`;
}

interface Notice {
  message?: string;
  error?: string;
  key?: string;
}

async function dashboard(env: Env, query: string, notice: Notice = {}): Promise<Response> {
  const [s, rows, failures] = await Promise.all([
    stats(env),
    search(env, query),
    recentFailures(env),
  ]);

  const tile = (n: number | string, label: string) =>
    `<div class="tile"><div class="n">${escapeHtml(n)}</div><div class="l">${escapeHtml(label)}</div></div>`;

  const failureTable = failures.length
    ? `<h2>Failed checkouts, last 7 days</h2>
       <div class="scroll"><table>
         <tr><th>When</th><th>Provider</th><th>Plan</th><th>Email</th><th>Reason</th></tr>
         ${failures
           .map(
             (f) => `<tr><td>${escapeHtml(date(f.created_at))}</td>
                         <td>${escapeHtml(f.provider)}</td>
                         <td>${escapeHtml(f.cycle)}</td>
                         <td>${escapeHtml(f.email ?? "—")}</td>
                         <td class="muted">${escapeHtml(f.failure ?? "")}</td></tr>`,
           )
           .join("")}
       </table></div>`
    : "";

  return page(
    "Admin — bubbleTranslate",
    `<h1>Subscribers</h1>
     ${notice.message ? `<p class="ok">${escapeHtml(notice.message)}</p>` : ""}
     ${notice.error ? `<p class="err">${escapeHtml(notice.error)}</p>` : ""}
     ${
       notice.key
         ? `<p class="warn">Give this to the customer. It is shown once and cannot be
             looked up again:</p><div class="key">${escapeHtml(notice.key)}</div>`
         : ""
     }

     <div class="tiles">
       ${tile(s.live, "live subscribers")}
       ${tile(s.monthly, "monthly")}
       ${tile(s.yearly, "yearly")}
       ${tile(s.paddle, "via Paddle")}
       ${tile(s.fresh, "new in 30 days")}
       ${tile(s.expiring, "ending in 7 days")}
       ${tile(s.winding_down, "cancelled, still paid")}
     </div>

     <form method="get" action="/admin" class="row">
       <div style="flex:1 1 260px">
         <label class="field" for="q">Search — email, licence key, <code>lc_…</code> id, or subscription ref</label>
         <input id="q" type="text" name="q" value="${escapeHtml(query)}"
                placeholder="musteri@ornek.com" spellcheck="false">
       </div>
       <button type="submit">Search</button>
     </form>

     <h2>Issue a licence</h2>
     <p class="muted">
       For the cases that never go through checkout: a press copy, a support
       apology, a beta tester. It creates a real licence with no payment behind
       it, marked <code>manual</code> so no Paddle webhook will ever move it and
       the account page offers it no cancel button. The key is shown once.
     </p>
     <form method="post" action="/admin/issue" class="row">
       <div style="flex:2 1 240px">
         <label class="field" for="issue-email">Email — for your records; delivery is up to you</label>
         <input id="issue-email" type="email" name="email" spellcheck="false"
                placeholder="gazeteci@ornek.com">
       </div>
       <div style="flex:0 1 150px">
         <label class="field" for="issue-cycle">Term</label>
         <select id="issue-cycle" name="cycle">
           <option value="yearly">Yearly</option>
           <option value="monthly">Monthly</option>
         </select>
       </div>
       <div style="flex:0 1 130px">
         <label class="field" for="issue-days">Days — overrides the term</label>
         <input id="issue-days" type="number" name="days" min="1" max="400" placeholder="365">
       </div>
       <div style="flex:0 1 110px">
         <label class="field" for="issue-seats">Devices</label>
         <input id="issue-seats" type="number" name="seats" min="1" max="20"
                value="${DEFAULT_SEATS}">
       </div>
       <button type="submit">Issue</button>
     </form>

     <h2>${query ? "Results" : `Latest ${PAGE_SIZE}`}</h2>
     ${rowsTable(rows)}
     ${failureTable}
     <hr>
     <p class="muted">
       Refunds and cancellations should normally be done in Paddle — its
       webhook updates this automatically. The Refund button here only marks the
       licence, and does not move any money. Support: ${escapeHtml(supportEmail(env))}
     </p>`,
    "",
    true,
  );
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
      return dashboard(env, id, {
        message: `Extended by ${days} days — now ends ${date(expiresAt)}.`,
      });
    }

    case "seats": {
      const { meta } = await env.DB.prepare("DELETE FROM seats WHERE licence_id = ?")
        .bind(licence.id)
        .run();
      return dashboard(env, id, {
        message: `Freed ${meta?.changes ?? 0} device slots. The customer can activate again.`,
      });
    }

    case "rotate": {
      const key = await rotateKey(env, licence);
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

  if (request.method === "GET" && pathname === "/admin") {
    return dashboard(env, url.searchParams.get("q") ?? "");
  }
  if (request.method === "POST") {
    const action = pathname.slice("/admin/".length);
    if (action === "issue") return issue(env, request);
    if (["extend", "seats", "rotate", "end", "update", "delete"].includes(action)) {
      return act(env, request, action);
    }
  }
  return new Response("Not found.", { status: 404 });
}
