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

import { type Env, supportEmail } from "./env";
import { type Licence, endLicence, isLive, licenceById, rotateKey } from "./licences";
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
      </tr>`,
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
    if (["extend", "seats", "rotate", "end"].includes(action)) {
      return act(env, request, action);
    }
  }
  return new Response("Not found.", { status: 404 });
}
