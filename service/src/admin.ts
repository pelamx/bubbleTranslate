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
import { reportPage } from "./report";
import { grantsAccess, subscriptionById } from "./mirror";
import { escapeHtml, page } from "./pages";
import { constantTimeEqual, now, sha256Hex } from "./tokens";

/** How many rows a listing shows. Enough to scan, few enough to render. */
const PAGE_SIZE = 40;

const DAY = 86_400;

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

interface Usage {
  daily: number;
  weekly: number;
  monthly: number;
  free_weekly: number;
  new_weekly: number;
  total: number;
}

/** Installs by their last daily ping. Counts people running the app, free and
 *  Pro alike; a download that was never opened is not in here. */
async function usage(env: Env): Promise<Usage> {
  const t = now();
  const row = await env.DB.prepare(
    `SELECT
       SUM(CASE WHEN last_seen > ?1 THEN 1 ELSE 0 END) AS daily,
       SUM(CASE WHEN last_seen > ?2 THEN 1 ELSE 0 END) AS weekly,
       SUM(CASE WHEN last_seen > ?3 THEN 1 ELSE 0 END) AS monthly,
       SUM(CASE WHEN last_seen > ?2 AND plan = 'free' THEN 1 ELSE 0 END) AS free_weekly,
       SUM(CASE WHEN first_seen > ?2 THEN 1 ELSE 0 END) AS new_weekly,
       COUNT(*) AS total
     FROM installs`,
  )
    .bind(t - 2 * DAY, t - 8 * DAY, t - 31 * DAY)
    .first<Usage>();
  return {
    daily: row?.daily ?? 0,
    weekly: row?.weekly ?? 0,
    monthly: row?.monthly ?? 0,
    free_weekly: row?.free_weekly ?? 0,
    new_weekly: row?.new_weekly ?? 0,
    total: row?.total ?? 0,
  };
}

/** The same installs the weekly tile counts, split by the OS each one reported
 *  on its last ping. Answers "a new person turned up — what are they running?"
 *  without naming any single machine. `os` is Rust's `env::consts::OS`, so the
 *  values are `macos` / `windows` / `linux`. */
async function osBreakdown(
  env: Env,
): Promise<{ os: string; active: number; fresh: number; today: number }[]> {
  const t = now();
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(os, 'unknown') AS os,
            SUM(CASE WHEN last_seen  > ?1 THEN 1 ELSE 0 END) AS active,
            SUM(CASE WHEN first_seen > ?1 THEN 1 ELSE 0 END) AS fresh,
            SUM(CASE WHEN first_seen > ?2 THEN 1 ELSE 0 END) AS today
       FROM installs
      WHERE last_seen > ?1 OR first_seen > ?1
      GROUP BY os
      ORDER BY active DESC, os ASC`,
  )
    .bind(t - 8 * DAY, t - DAY)
    .all<{ os: string; active: number; fresh: number; today: number }>();
  return results ?? [];
}

/** How the app names an OS to how a person reads it. An unlisted value (a
 *  phone build, a BSD) is shown as it arrived rather than hidden. */
function osLabel(os: string): string {
  const known: Record<string, string> = {
    macos: "macOS",
    windows: "Windows",
    linux: "Linux",
    unknown: "Unknown",
  };
  return known[os] ?? os;
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

  // SUM over an empty table is NULL, not 0.
  return {
    live: row?.live ?? 0,
    monthly: row?.monthly ?? 0,
    yearly: row?.yearly ?? 0,
    paddle: row?.paddle ?? 0,
    winding_down: row?.winding_down ?? 0,
    expiring: row?.expiring ?? 0,
    fresh: row?.fresh ?? 0,
    total: row?.total ?? 0,
  };
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

interface Pulse {
  installs_today: number;
  installs_yesterday: number;
  active_today: number;
  subs_today: number;
  subs_yesterday: number;
  /** New installs per day, oldest first; the last entry is the last 24 hours. */
  series: number[];
}

/** What changed: the last 24 hours against the 24 before them. "Today" is a
 *  rolling day rather than a calendar one, so the comparison is never between
 *  a full yesterday and a morning. */
async function pulse(env: Env): Promise<Pulse> {
  const t = now();
  const [inst, subs, days] = await Promise.all([
    env.DB.prepare(
      `SELECT
         SUM(CASE WHEN first_seen > ?1 THEN 1 ELSE 0 END) AS today,
         SUM(CASE WHEN first_seen > ?2 AND first_seen <= ?1 THEN 1 ELSE 0 END) AS yesterday,
         SUM(CASE WHEN last_seen  > ?1 THEN 1 ELSE 0 END) AS active
       FROM installs`,
    )
      .bind(t - DAY, t - 2 * DAY)
      .first<{ today: number; yesterday: number; active: number }>(),
    env.DB.prepare(
      `SELECT
         SUM(CASE WHEN created_at > ?1 THEN 1 ELSE 0 END) AS today,
         SUM(CASE WHEN created_at > ?2 AND created_at <= ?1 THEN 1 ELSE 0 END) AS yesterday
       FROM licences`,
    )
      .bind(t - DAY, t - 2 * DAY)
      .first<{ today: number; yesterday: number }>(),
    env.DB.prepare(
      `SELECT CAST((?1 - first_seen) / ${DAY} AS INTEGER) AS ago, COUNT(*) AS n
         FROM installs WHERE first_seen > ?1 - 14 * ${DAY}
        GROUP BY ago`,
    )
      .bind(t)
      .all<{ ago: number; n: number }>(),
  ]);
  const series = new Array<number>(14).fill(0);
  for (const d of days.results ?? []) {
    if (d.ago >= 0 && d.ago < 14) series[13 - d.ago] = d.n;
  }
  return {
    installs_today: inst?.today ?? 0,
    installs_yesterday: inst?.yesterday ?? 0,
    active_today: inst?.active ?? 0,
    subs_today: subs?.today ?? 0,
    subs_yesterday: subs?.yesterday ?? 0,
    series,
  };
}

/** The operator panel's own look, kept to this page. Injected through
 *  `page()`'s `head`, after the shared stylesheet, so it overrides the base
 *  without touching the buyer-facing pages. Three layers, loudest first:
 *  what changed since yesterday, where things stand, and the tools. */
const ADMIN_STYLE = `
  :root {
    --bg:#0f1117; --card:#171a23; --card-2:#1d212c; --border:#262b38;
    --text:#e6e8ef; --muted:#8a91a5; --faint:#5b6275;
    --brand:#6ea8fe; --accent:#4ade80; --red:#f87171; --orange:#fb923c;
  }
  body {
    font-family: Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
    color: var(--text); background: var(--bg);
  }
  .sheet { background: transparent; border: 0; box-shadow: none; padding: 0 0 40px; }
  .topbar { display: flex; align-items: center; gap: 12px; margin: 8px 0 22px; }
  .topbar h1 { font-size: 20px; font-weight: 700; margin: 0; }
  .topbar .spacer { flex: 1; }
  .pill { font-size: 11px; color: var(--muted); border: 1px solid var(--border); border-radius: 999px; padding: 3px 10px; }
  .pill.prod { color: var(--accent); border-color: rgba(74,222,128,.4); }
  .label { font-size: 12px; font-weight: 600; color: var(--muted); text-transform: uppercase; letter-spacing: .6px; margin: 0 0 10px; }
  .card { background: var(--card); border: 1px solid var(--border); border-radius: 14px; padding: 18px 20px; margin-bottom: 16px; }
  .grid { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); }
  .stat { background: var(--card-2); border-radius: 10px; padding: 14px 16px; }
  .stat .k { font-size: 13px; color: var(--muted); }
  .stat .v { font-size: 32px; font-weight: 700; font-variant-numeric: tabular-nums; line-height: 1.2; margin-top: 2px; }
  .stat .d { font-size: 12.5px; color: var(--muted); margin-top: 4px; }
  .stat .sub { font-size: 12.5px; color: var(--faint); margin-top: 6px; }
  .up { color: var(--accent); } .down { color: var(--red); }
  .stat.alert { box-shadow: inset 3px 0 0 var(--orange); }
  .stat.alert .v { color: var(--orange); }
  .os { display: inline-block; font-size: 12px; background: var(--bg); border-radius: 6px; padding: 2px 7px; margin: 6px 4px 0 0; color: var(--text); }
  .bars { display: flex; align-items: flex-end; gap: 4px; height: 70px; margin-top: 12px; }
  .bars div { flex: 1; background: var(--brand); opacity: .75; border-radius: 3px 3px 0 0; min-height: 2px; }
  .bars div:last-child { opacity: 1; }
  .bars-axis { display: flex; justify-content: space-between; font-size: 11px; color: var(--faint); margin-top: 4px; }
  details.card { padding: 0; }
  details.card > summary { cursor: pointer; padding: 15px 20px; font-weight: 600; list-style: none; }
  details.card > summary::-webkit-details-marker { display: none; }
  details.card > summary::before { content: "▸ "; color: var(--muted); }
  details.card[open] > summary::before { content: "▾ "; }
  details.card > .body { padding: 0 20px 18px; }
  details.card.bad > summary { color: var(--red); }
  table { font-size: 13px; }
  th, td { border-bottom: 1px solid var(--border); padding: 8px 10px; }
  th { color: var(--muted); font-size: 11.5px; font-weight: 600; }
  table tr:hover td { background: rgba(110,168,254,.05); }
  code { color: var(--brand); background: rgba(110,168,254,.1); padding: 1px 5px; border-radius: 5px; }
  button {
    font-family: inherit; font-weight: 600; font-size: 13px; color: #0f1117;
    background: var(--brand); border: 0; border-radius: 8px; padding: 9px 14px; cursor: pointer;
  }
  button:hover:not(:disabled) { filter: brightness(1.1); }
  button.quiet { background: transparent; border: 1px solid var(--border); color: var(--text); }
  button.danger { background: var(--red); }
  form.inline button { font-size: 12px; padding: 5px 10px; }
  input, select, textarea {
    font-family: inherit; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px; font-size: 14px;
  }
  input:focus, select:focus, textarea:focus { outline: none; border-color: var(--brand); }
  label.field { color: var(--muted); font-size: 12px; }
  .ok { color: var(--accent); } .err { color: var(--red); } .warn { color: var(--orange); }
  .muted { color: var(--muted); }
  .key { color: var(--brand); background: var(--bg); border: 1px dashed var(--border); border-radius: 8px; padding: 12px 14px; letter-spacing: 1px; font-family: ui-monospace, monospace; }
  .foot { font-size: 12.5px; }
`;

/** "▲ +4 (dün 8)" — today against yesterday, coloured by direction. */
function delta(today: number, yesterday: number): string {
  const d = today - yesterday;
  const cls = d > 0 ? "up" : d < 0 ? "down" : "";
  const arrow = d > 0 ? "▲" : d < 0 ? "▼" : "•";
  return `<div class="d"><span class="${cls}">${arrow} ${d > 0 ? "+" : ""}${d}</span> · yesterday ${yesterday}</div>`;
}

async function dashboard(env: Env, query: string, notice: Notice = {}): Promise<Response> {
  const [s, u, p, byOs, rows, failures] = await Promise.all([
    stats(env),
    usage(env),
    pulse(env),
    osBreakdown(env),
    search(env, query),
    recentFailures(env),
  ]);

  const prod = env.PADDLE_ENV === "production";
  const osChips = (pick: (o: { os: string; active: number; fresh: number; today: number }) => number) =>
    byOs
      .filter((o) => pick(o) > 0)
      .map((o) => `<span class="os">${escapeHtml(osLabel(o.os))} ${pick(o)}</span>`)
      .join("");
  const peak = Math.max(1, ...p.series);
  const bars = p.series
    .map((n, i) => {
      const ago = 13 - i;
      const when = ago === 0 ? "last 24h" : `${ago} days ago`;
      return `<div style="height:${Math.round((n / peak) * 100)}%" title="${when}: ${n}"></div>`;
    })
    .join("");

  const failureTable = failures.length
    ? `<details class="card bad"><summary>Failed checkouts, last 7 days (${failures.length})</summary>
       <div class="body scroll"><table>
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
       </table></div></details>`
    : "";

  return page(
    "Admin — bubbleTranslate",
    `<div class="topbar">
       <h1>bubbleTranslate admin</h1><span class="spacer"></span>
       <span class="pill${prod ? " prod" : ""}">${escapeHtml(env.PADDLE_ENV ?? "sandbox")}</span>
     </div>
     ${notice.message ? `<p class="ok">${escapeHtml(notice.message)}</p>` : ""}
     ${notice.error ? `<p class="err">${escapeHtml(notice.error)}</p>` : ""}
     ${
       notice.key
         ? `<p class="warn">Give this to the customer. It is shown once and cannot be
             looked up again:</p><div class="key">${escapeHtml(notice.key)}</div>`
         : ""
     }

     <section class="card">
       <p class="label">Last 24 hours</p>
       <div class="grid">
         <div class="stat"><div class="k">New installs</div><div class="v">${p.installs_today}</div>
           ${delta(p.installs_today, p.installs_yesterday)}
           <div>${osChips((o) => o.today)}</div></div>
         <div class="stat"><div class="k">New subscribers</div><div class="v">${p.subs_today}</div>
           ${delta(p.subs_today, p.subs_yesterday)}</div>
         <div class="stat"><div class="k">Active installs</div><div class="v">${p.active_today}</div>
           <div class="sub">opened the app in the last 24h</div></div>
       </div>
     </section>

     <section class="card">
       <p class="label">Overall</p>
       <div class="grid">
         <div class="stat"><div class="k">Live subscribers</div><div class="v">${s.live}</div>
           <div class="sub">${s.monthly} monthly · ${s.yearly} yearly · ${s.fresh} new in 30d</div></div>
         <div class="stat"><div class="k">Active this week</div><div class="v">${u.weekly}</div>
           <div>${osChips((o) => o.active)}</div>
           <div class="sub">${u.free_weekly} free · ${u.new_weekly} new this week</div></div>
         <div class="stat"><div class="k">Installs ever seen</div><div class="v">${u.total}</div>
           <div class="sub">${u.monthly} active this month</div></div>
         <div class="stat${s.expiring > 0 ? " alert" : ""}"><div class="k">Ending in 7 days</div><div class="v">${s.expiring}</div>
           <div class="sub">${s.winding_down} cancelled, still paid</div></div>
       </div>
       <p class="label" style="margin-top:20px">New installs, last 14 days</p>
       <div class="bars">${bars}</div>
       <div class="bars-axis"><span>14 days ago</span><span>today</span></div>
     </section>

     ${failureTable}

     <details class="card"${query ? " open" : ""}>
       <summary>Licences${query ? ` — results for “${escapeHtml(query)}”` : ""}</summary>
       <div class="body">
         <form method="get" action="/admin" class="row">
           <div style="flex:1 1 260px">
             <label class="field" for="q">Search — email, licence key, <code>lc_…</code> id, or subscription ref</label>
             <input id="q" type="text" name="q" value="${escapeHtml(query)}"
                    placeholder="musteri@ornek.com" spellcheck="false">
           </div>
           <button type="submit">Search</button>
         </form>
         <p class="muted">${query ? "Results" : `Latest ${PAGE_SIZE}`}</p>
         ${rowsTable(rows)}
       </div>
     </details>

     <details class="card">
       <summary>Issue a licence</summary>
       <div class="body">
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
       </div>
     </details>

     <p class="muted foot">
       Refunds and cancellations should normally be done in Paddle — its
       webhook updates this automatically. The Refund button here only marks the
       licence, and does not move any money. Support: ${escapeHtml(supportEmail(env))}
     </p>`,
    `<style>${ADMIN_STYLE}</style>`,
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

  if (request.method === "GET" && pathname === "/admin/report") {
    return reportPage(env);
  }
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
