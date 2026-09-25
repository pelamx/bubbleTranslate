// What the panel draws: the dashboard page and its tables.

import { type Env, USD_AMOUNT, supportEmail } from "../env";
import { DEFAULT_SEATS, isLive } from "../licences";
import { escapeHtml, page } from "../pages";
import { now } from "../tokens";
import { DAY, PAGE_SIZE, date, startOfToday } from "./shared";
import {
  HealthRow,
  LicenceOsRow,
  NOT_MINE_INSTALL,
  NOT_MINE_LICENCE,
  OsRow,
  PLATFORMS,
  Row,
  type AdminLogRow,
  type Downloads,
  type EndingRow,
  type Revenue,
  type WebhookRow,
  adminLog,
  endingSoon,
  proShare,
  renews,
  revenue,
  webhookEvents,
  UserRow,
  compareVersions,
  downloads,
  health,
  ignoreList,
  licenceOs,
  mineInstalls,
  mineBreakdown,
  type ProviderHealthRow,
  providerHealth,
  osBreakdown,
  osLabel,
  published,
  ratio,
  recentFailures,
  search,
  stats,
  usage,
  users,
  versions,
} from "./queries";

export function ago(unix: number): string {
  const s = Math.max(0, now() - unix);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < DAY) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / DAY)}d ago`;
}

export function usersTable(list: UserRow[]): string {
  if (!list.length) return `<p class="muted">No installs in the last 30 days.</p>`;
  const t = now();
  const body = list
    .map((u) => {
      const mine = u.mine === 1;
      const badge = mine
        ? `<span class="badge you">you</span>`
        : u.first_seen >= startOfToday()
          ? `<span class="badge new">new</span>`
          : u.last_seen > t - 2 * DAY
            ? `<span class="badge on">active</span>`
            : u.last_seen > t - 8 * DAY
              ? `<span class="badge">this week</span>`
              : `<span class="badge off">idle</span>`;
      return `<tr${mine ? ' class="mine"' : ""}>
        <td>${badge}</td>
        <td>${escapeHtml(osLabel(u.os ?? "unknown"))}</td>
        <td>${escapeHtml(u.app ?? "—")}</td>
        <td>${escapeHtml(u.plan ?? "—")}</td>
        <td title="${escapeHtml(date(u.first_seen))}">${ago(u.first_seen)}</td>
        <td title="${escapeHtml(date(u.last_seen))}">${ago(u.last_seen)}</td>
        <td><code>${escapeHtml(u.install.slice(0, 8))}</code></td>
        <td><form method="post" action="/admin/mine" class="inline">
          <input type="hidden" name="install" value="${escapeHtml(u.install)}">
          <input type="hidden" name="mine" value="${mine ? "0" : "1"}">
          <button class="quiet" type="submit">${mine ? "Not me" : "This is me"}</button>
        </form></td>
      </tr>`;
    })
    .join("");
  return `<div class="scroll"><table>
    <tr><th></th><th>OS</th><th>Version</th><th>Plan</th><th>First seen</th><th>Last seen</th><th>Id</th><th></th></tr>
    ${body}
  </table></div>`;
}
/** An `onclick` asking the operator to confirm, naming the licence and its
 *  owner so a slip of the mouse onto the neighbouring row is caught. The
 *  message goes through JSON for the script and escapeHtml for the attribute,
 *  since an email address can hold a quote. */
function confirmFor(r: Row, what: string): string {
  const who = `${r.id}${r.email ? ` (${r.email})` : ""}`;
  return `onclick="return confirm(${escapeHtml(JSON.stringify(`${what}\n\n${who}`))})"`;
}

export function relativeDays(unix: number): string {
  const days = Math.round((unix - now()) / DAY);
  if (days < 0) return `${-days}d ago`;
  if (days === 0) return "today";
  return `in ${days}d`;
}

/** Whether a licence was sold through Paddle, as opposed to issued by hand.
 *  The two are kept apart on the panel because they are not the same thing:
 *  one has a customer, a payment and webhooks behind it, the other has none. */
export function isPaddle(licence: Row): boolean {
  return licence.provider === "paddle";
}

export function statusCell(licence: Row): string {
  if (!isLive(licence)) {
    // A manual licence was never paid for, so ending one early is a
    // revocation, whatever the column underneath calls it.
    const ended = licence.status === "refunded" ? (isPaddle(licence) ? "refunded" : "revoked") : "expired";
    return `<span class="err">${escapeHtml(ended)}</span>`;
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
export function editPanel(r: Row): string {
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
          ${option("active", "Active", r.status)}${option("cancelled", "Cancelled", r.status)}${option("refunded", isPaddle(r) ? "Refunded" : "Revoked", r.status)}
        </select>
      </div>
      <div style="flex:0 1 130px">
        <label class="field" for="e-lim-${escapeHtml(r.id)}">Translations</label>
        <input id="e-lim-${escapeHtml(r.id)}" type="number" name="translation_limit" min="0"
               value="${r.translation_limit ?? ""}">
      </div>
      <button type="submit" ${confirmFor(r, "Save these changes?")}>Save</button>
    </form>
    <form method="post" action="/admin/delete" class="row">
      <input type="hidden" name="id" value="${escapeHtml(r.id)}">
      <button class="danger" type="submit"
        ${confirmFor(r, "Delete this licence and its devices for good? The key stops working and nothing here can bring it back.")}>Delete this licence</button>
    </form>
    <p class="muted">
      Blank translations means unlimited, which is what a paid licence is.
      Created ${escapeHtml(date(r.created_at))} · provider <code>${escapeHtml(r.provider)}</code>${
        r.provider_ref ? ` · ref <code>${escapeHtml(r.provider_ref)}</code>` : ""
      }
    </p>
  </details>`;
}

/** The licences of one kind, under a heading that says which. */
export function licenceGroup(rows: Row[], paddle: boolean, searching: boolean): string {
  const kind = paddle ? "paddle" : "manual";
  const title = paddle ? "Paddle — sold" : "Manual — issued by hand";
  const note = paddle
    ? "Bought through checkout. Paddle's webhooks keep these current; refunds and cancellations belong in Paddle."
    : "Press copies, testers, support apologies. No payment and no webhook behind them — only this panel changes them.";
  const empty = searching ? "Nothing matched." : paddle ? "No licence has been sold yet." : "None issued yet.";
  return `<section class="group ${kind}">
    <p class="group-head"><span class="badge ${kind}">${paddle ? "Paddle" : "Manual"}</span>
      <b>${escapeHtml(title)}</b> <span class="muted">${rows.length}${
        !searching && rows.length >= PAGE_SIZE ? ` latest` : ""
      }</span></p>
    <p class="muted group-note">${escapeHtml(note)}</p>
    ${rows.length ? rowsTable(rows) : `<p class="muted">${escapeHtml(empty)}</p>`}
  </section>`;
}

export function rowsTable(rows: Row[]): string {
  if (!rows.length) return '<p class="muted">Nothing matched.</p>';
  const body = rows
    .map(
      (r) => `
      <tr>
        <td><code>${escapeHtml(r.id)}</code>${r.mine ? ` <span class="badge you">you</span>` : ""}</td>
        <td>${escapeHtml(r.email ?? "—")}</td>
        <td>${escapeHtml(r.cycle)}${
          isPaddle(r) && r.provider_ref
            ? `<br><span class="muted" title="Paddle subscription"><code>${escapeHtml(r.provider_ref)}</code></span>`
            : ""
        }</td>
        <td>${statusCell(r)}</td>
        <td>${escapeHtml(date(r.expires_at))}<br><span class="muted">${escapeHtml(
          relativeDays(r.expires_at),
        )}</span></td>
        <td class="num">${r.seats}/${r.seat_limit}</td>
        <td>
          <form class="inline" method="post" action="/admin/mine-licence">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <input type="hidden" name="mine" value="${r.mine ? "0" : "1"}">
            <button class="quiet" type="submit">${r.mine ? "Not me" : "This is me"}</button>
          </form>
          <form class="inline" method="post" action="/admin/extend">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <input type="hidden" name="days" value="30">
            <button class="quiet" type="submit" ${confirmFor(r, "Add 30 days to this licence?")}>+30d</button>
          </form>
          <form class="inline" method="post" action="/admin/seats">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <button class="quiet" type="submit"
              ${confirmFor(r, "Free all devices on this licence?")}>Free seats</button>
          </form>
          <form class="inline" method="post" action="/admin/rotate">
            <input type="hidden" name="id" value="${escapeHtml(r.id)}">
            <button class="quiet" type="submit"
              ${confirmFor(r, "Issue a new key? The old one stops working immediately.")}>New key</button>
          </form>
          ${
            isLive(r) && r.status !== "cancelled"
              ? `<form class="inline" method="post" action="/admin/end">
                   <input type="hidden" name="id" value="${escapeHtml(r.id)}">
                   <input type="hidden" name="status" value="refunded">
                   ${
                     isPaddle(r)
                       ? `<button class="quiet" type="submit"
                     ${confirmFor(r, "Mark refunded? This ends Pro immediately. It does not move any money -- refund in Paddle.")}>Refund</button>`
                       : `<button class="quiet" type="submit"
                     ${confirmFor(r, "Revoke this key? This ends Pro immediately.")}>Revoke</button>`
                   }
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

export interface Notice {
  message?: string;
  error?: string;
  key?: string;
}

export interface Pulse {
  installs_today: number;
  installs_yesterday: number;
  active_today: number;
  subs_today: number;
  subs_yesterday: number;
  /** New installs per day, oldest first; the last entry is today. */
  series: number[];
}

/** What changed: today against yesterday, as calendar days in local time. */
export async function pulse(env: Env): Promise<Pulse> {
  const t = startOfToday();
  const [inst, subs, days] = await Promise.all([
    env.DB.prepare(
      `SELECT
         SUM(CASE WHEN first_seen >= ?1 THEN 1 ELSE 0 END) AS today,
         SUM(CASE WHEN first_seen >= ?2 AND first_seen < ?1 THEN 1 ELSE 0 END) AS yesterday,
         SUM(CASE WHEN last_seen  >= ?1 THEN 1 ELSE 0 END) AS active
       FROM installs WHERE ${NOT_MINE_INSTALL}`,
    )
      .bind(t, t - DAY, null, null, null, null, null, null, await mineInstalls(env))
      .first<{ today: number; yesterday: number; active: number }>(),
    env.DB.prepare(
      `SELECT
         SUM(CASE WHEN created_at >= ?1 THEN 1 ELSE 0 END) AS today,
         SUM(CASE WHEN created_at >= ?2 AND created_at < ?1 THEN 1 ELSE 0 END) AS yesterday
       FROM licences WHERE provider = 'paddle' AND ${NOT_MINE_LICENCE}`,
    )
      .bind(t, t - DAY, null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_EMAILS))
      .first<{ today: number; yesterday: number }>(),
    env.DB.prepare(
      // Days before today: 0 for anything since midnight, 1 for yesterday.
      `SELECT CAST((?1 + ${DAY} - 1 - first_seen) / ${DAY} AS INTEGER) AS ago, COUNT(*) AS n
         FROM installs WHERE first_seen >= ?1 - 13 * ${DAY} AND ${NOT_MINE_INSTALL}
        GROUP BY ago`,
    )
      .bind(t, null, null, null, null, null, null, null, await mineInstalls(env))
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
export const ADMIN_STYLE = `
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
  .badge { font-size: 11px; border-radius: 999px; padding: 2px 8px; background: var(--card-2); color: var(--muted); white-space: nowrap; }
  .badge.new { background: rgba(110,168,254,.18); color: var(--brand); }
  .badge.on { background: rgba(74,222,128,.15); color: var(--accent); }
  .badge.off { color: var(--faint); }
  .badge.you { background: rgba(251,146,60,.15); color: var(--orange); }
  .badge.paddle { background: rgba(110,168,254,.18); color: var(--brand); }
  .badge.manual { background: rgba(251,146,60,.15); color: var(--orange); }
  .group { border-left: 3px solid var(--border); padding-left: 14px; margin-top: 22px; }
  .group.paddle { border-left-color: var(--brand); }
  .group.manual { border-left-color: var(--orange); }
  .group-head { margin: 0; display: flex; align-items: center; gap: 8px; }
  .group-note { margin: 4px 0 10px; }
  tr.mine td { opacity: .6; }
  tr.total td { border-top: 1px solid var(--border); font-weight: 600; }
  .os.zero { color: var(--faint); }
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
  details.card.warnbox > summary { color: var(--orange); }
  a code { text-decoration: none; }
  .topbar a.pill { text-decoration: none; }
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

const usd = (n: number) => `$${Number.isInteger(n) ? n : n.toFixed(2)}`;

/** Recurring revenue, what is set to renew, and six months of new against
 *  lost. List prices in USD, as the weekly report counts them. */
function backendsCard(rows: ProviderHealthRow[]): string {
  if (rows.length === 0) {
    return `<section class="card">
       <p class="label">Translation backends, last 7 days</p>
       <p class="sub">Nothing reported yet. Copies running a build older than 0.3.6 do not send this.</p>
     </section>`;
  }
  const total = rows.reduce((a, r) => a + r.ok, 0);
  // The fallbacks answering at all is the tell: when the first backend is
  // healthy they win nothing, so a share worth noticing means it is refusing.
  const leader = rows[0]?.provider ?? "";
  const fallback = rows.slice(1).reduce((a, r) => a + r.ok, 0);
  const worry = total > 0 && fallback / total > 0.1;
  return `<section class="card">
       <p class="label">Translation backends, last 7 days</p>
       <div class="scroll"><table>
         <tr><th>Backend</th><th class="num">Answered</th><th class="num">Failed</th>
             <th class="num">Share</th><th class="num">Reports</th></tr>
         ${rows
           .map(
             (r) => `<tr><td>${escapeHtml(r.provider)}</td>
               <td class="num">${r.ok}</td>
               <td class="num">${r.failed > 0 ? `<b>${r.failed}</b>` : "0"}</td>
               <td class="num">${total > 0 ? Math.round((r.ok / total) * 100) : 0}%</td>
               <td class="num">${r.reports}</td></tr>`,
           )
           .join("")}
       </table></div>
       <p class="sub"${worry ? ' style="color:#b45309"' : ""}>${
         worry
           ? `The fallbacks are answering ${Math.round((fallback / total) * 100)}% of translations — ${escapeHtml(leader)} is refusing more than it should.`
           : `${escapeHtml(leader)} is answering ${Math.round(((rows[0]?.ok ?? 0) / Math.max(total, 1)) * 100)}% of translations, which is what a healthy chain looks like.`
       }</p>
     </section>`;
}

export function revenueCard(m: Revenue, share: { pro: number; active: number }): string {
  const rows = m.months
    .map(
      (r) => `<tr><td>${escapeHtml(r.month)}</td>
        <td class="num">${r.new_monthly}</td><td class="num">${r.new_yearly}</td>
        <td class="num">${usd(r.new_monthly * USD_AMOUNT.monthly + r.new_yearly * USD_AMOUNT.yearly)}</td>
        <td class="num">${r.cancelled || `<span class="muted">0</span>`}</td>
        <td class="num">${r.refunded || `<span class="muted">0</span>`}</td></tr>`,
    )
    .join("");
  return `<section class="card" id="revenue">
    <p class="label">Revenue <span style="text-transform:none;font-weight:400">(list prices, USD)</span></p>
    <div class="grid">
      <div class="stat"><div class="k">Monthly recurring</div><div class="v">${usd(m.mrr)}</div>
        <div class="sub">${m.renewing_monthly} monthly · ${m.renewing_yearly} yearly set to renew</div></div>
      <div class="stat"><div class="k">Yearly run rate</div><div class="v">${usd(m.mrr * 12)}</div>
        <div class="sub">monthly recurring × 12</div></div>
      <div class="stat"><div class="k">On Pro this week</div><div class="v">${
        share.active ? `${Math.round((share.pro / share.active) * 100)}%` : "—"
      }</div><div class="sub">${share.pro} of ${share.active} active installs</div></div>
    </div>
    <div class="scroll" style="margin-top:16px"><table>
      <tr><th>Month</th><th class="num">New monthly</th><th class="num">New yearly</th>
          <th class="num">New sales</th><th class="num">Cancelled</th><th class="num">Refunded</th></tr>
      ${rows}
    </table></div>
    <p class="muted foot">Renewals are not counted as new sales. Paddle charges each buyer in their
      own currency with tax added, so what it pays out differs from these figures.</p>
  </section>`;
}

/** Live licences whose term runs out within 30 days, with whether Paddle is
 *  going to renew each one — the list to act on before they lapse. */
export function endingCard(list: EndingRow[]): string {
  const label = { yes: `<span class="ok">renews</span>`, no: `<span class="warn">won't renew</span>`,
    unknown: `<span class="muted">unknown</span>` };
  const lapsing = list.filter((r) => renews(r) !== "yes").length;
  const body = list
    .map(
      (r) => `<tr${r.mine ? ' class="mine"' : ""}>
        <td><a href="/admin?q=${encodeURIComponent(r.id)}#licences"><code>${escapeHtml(r.id)}</code></a>${
          r.mine ? ` <span class="badge you">you</span>` : ""
        }</td>
        <td>${escapeHtml(r.email ?? "—")}</td>
        <td><span class="badge ${isPaddle(r) ? "paddle" : "manual"}">${isPaddle(r) ? "Paddle" : "Manual"}</span> ${escapeHtml(r.cycle)}</td>
        <td>${escapeHtml(date(r.expires_at))} <span class="muted">${escapeHtml(relativeDays(r.expires_at))}</span></td>
        <td>${label[renews(r)]}</td></tr>`,
    )
    .join("");
  return `<details class="card${lapsing ? " warnbox" : ""}" id="ending">
    <summary>Ending in the next 30 days (${list.length}${lapsing ? ` · ${lapsing} won't renew` : ""})</summary>
    <div class="body">${
      list.length
        ? `<div class="scroll"><table><tr><th>Licence</th><th>Email</th><th>Kind</th><th>Ends</th><th>Renews?</th></tr>${body}</table></div>
           <p class="muted foot"><b>Unknown</b>: no webhook has described its subscription yet, so Paddle's plans for it are not known here.</p>`
        : `<p class="muted">Nothing ends in the next 30 days.</p>`
    }</div>
  </details>`;
}

/** The latest Paddle deliveries. Open, and red, when any in the last week
 *  was refused, unsigned or failed — the cases where a payment can go missing. */
export function webhookCard(hooks: { rows: WebhookRow[]; bad: number }): string {
  const cls: Record<string, string> = { handled: "ok", ignored: "muted" };
  const body = hooks.rows
    .map(
      (w) => `<tr><td title="${escapeHtml(new Date(w.at * 1000).toISOString())}">${ago(w.at)}</td>
        <td>${escapeHtml(w.event_type ?? "—")}</td>
        <td>${w.entity_id ? `<code>${escapeHtml(w.entity_id)}</code>` : "—"}</td>
        <td class="${cls[w.outcome] ?? "err"}">${escapeHtml(w.outcome)}</td>
        <td class="muted">${escapeHtml(w.detail ?? "")}</td></tr>`,
    )
    .join("");
  return `<details class="card${hooks.bad ? " bad" : ""}" id="webhooks"${hooks.bad ? " open" : ""}>
    <summary>Paddle webhooks${hooks.bad ? ` — ${hooks.bad} failed in the last 7 days` : ""}</summary>
    <div class="body">${
      hooks.rows.length
        ? `<div class="scroll"><table><tr><th>When</th><th>Event</th><th>Paddle id</th><th>Outcome</th><th>Detail</th></tr>${body}</table></div>
           <p class="muted foot"><b>Ignored</b> is normal: Paddle sends events nothing here needs.
             <b>Refused</b>, <b>bad signature</b> and <b>error</b> are not — Paddle retries those, and a
             run of them means a payment is not reaching a licence.</p>`
        : `<p class="muted">No deliveries recorded yet.</p>`
    }</div>
  </details>`;
}

/** What the operator did here, newest first. */
export function historyCard(log: AdminLogRow[]): string {
  const body = log
    .map(
      (e) => `<tr><td title="${escapeHtml(new Date(e.at * 1000).toISOString())}">${ago(e.at)}</td>
        <td>${escapeHtml(e.action)}</td>
        <td>${e.licence_id ? `<a href="/admin?q=${encodeURIComponent(e.licence_id)}#licences"><code>${escapeHtml(e.licence_id)}</code></a>` : "—"}</td>
        <td class="muted">${escapeHtml(e.detail ?? "")}</td></tr>`,
    )
    .join("");
  return `<details class="card" id="history">
    <summary>Your actions (${log.length} latest)</summary>
    <div class="body">${
      log.length
        ? `<div class="scroll"><table><tr><th>When</th><th>Action</th><th>Licence</th><th>What changed</th></tr>${body}</table></div>`
        : `<p class="muted">Nothing done from this panel yet.</p>`
    }</div>
  </details>`;
}

/** Downloads of each release, per platform, newest first. */
export function releasesTable(dl: Downloads | null): string {
  if (!dl) return `<p class="muted">GitHub could not be reached just now.</p>`;
  if (!dl.releases.length) return `<p class="muted">No releases yet.</p>`;
  const cell = (n: number) => `<td class="num">${n || `<span class="muted">—</span>`}</td>`;
  return `<div class="scroll"><table>
    <tr><th>Release</th><th>Published</th>${PLATFORMS.map((os) => `<th class="num">${osLabel(os)}</th>`).join("")}<th class="num">Total</th></tr>
    ${dl.releases
      .slice(0, 10)
      .map(
        (r) => `<tr><td>${escapeHtml(r.tag)}</td><td class="muted">${escapeHtml(r.published)}</td>
          ${PLATFORMS.map((os) => cell(r[os])).join("")}
          <td class="num"><b>${r.windows + r.linux + r.macos}</b></td></tr>`,
      )
      .join("")}
  </table></div>`;
}

/** "▲ +4 (dün 8)" — today against yesterday, coloured by direction. */
export function delta(today: number, yesterday: number): string {
  const d = today - yesterday;
  const cls = d > 0 ? "up" : d < 0 ? "down" : "";
  const arrow = d > 0 ? "▲" : d < 0 ? "▼" : "•";
  return `<div class="d"><span class="${cls}">${arrow} ${d > 0 ? "+" : ""}${d}</span> · yesterday ${yesterday}</div>`;
}

export async function dashboard(env: Env, query: string, notice: Notice = {}): Promise<Response> {
  const [s, u, p, byOs, mineOs, licOs, rows, failures, people, hl, vers, downloadsNow, pub, log, hooks, ending, money, share, backends] = await Promise.all([
    stats(env),
    usage(env),
    pulse(env),
    osBreakdown(env),
    mineBreakdown(env),
    licenceOs(env),
    search(env, query),
    recentFailures(env),
    users(env),
    health(env),
    versions(env),
    downloads(env),
    published(),
    adminLog(env),
    webhookEvents(env),
    endingSoon(env),
    revenue(env),
    proShare(env),
    providerHealth(env),
  ]);
  const dl = downloadsNow?.total ?? null;

  const h = (os: string) =>
    hl.find((x) => x.os === os) ?? { os, total: 0, day_base: 0, day_back: 0, week_base: 0, week_back: 0, lost: 0 };
  const sum = (f: (r: HealthRow) => number) => hl.reduce((a, r) => a + f(r), 0);
  const healthRows = [...PLATFORMS.map((os) => ({ label: osLabel(os), r: h(os), dl: dl?.[os] })),
    { label: "All", r: { os: "all", total: sum((r) => r.total), day_base: sum((r) => r.day_base),
        day_back: sum((r) => r.day_back), week_base: sum((r) => r.week_base),
        week_back: sum((r) => r.week_back), lost: sum((r) => r.lost) },
      dl: dl ? PLATFORMS.reduce((a, os) => a + dl[os], 0) : undefined }]
    .map(({ label, r, dl: d }) => `<tr${label === "All" ? ' class="total"' : ""}>
        <td>${escapeHtml(label)}</td>
        <td class="num">${d ?? "—"}</td>
        <td class="num">${r.total}</td>
        <td>${d ? ratio(Math.min(r.total, d), d) : `<span class="muted">—</span>`}</td>
        <td>${ratio(r.day_back, r.day_base)}</td>
        <td>${ratio(r.week_back, r.week_base)}</td>
        <td class="num">${r.lost}</td>
      </tr>`)
    .join("");

  // "Latest" is what each platform is offered today, not the highest number
  // anyone happens to run: the platforms are released separately. Only
  // versions somebody runs get a row; the published one is named in each
  // column's header, so a release nobody has installed yet is still visible.
  const versionList = [...new Set(vers.map((v) => v.app))].sort(compareVersions);
  const versionRows = versionList
    .map((app) => {
      const n = (os: string) => vers.filter((v) => v.app === app && v.os === os).reduce((a, v) => a + v.n, 0);
      const all = vers.filter((v) => v.app === app).reduce((a, v) => a + v.n, 0);
      // No "latest" on the row itself: a version can be current on one
      // platform and behind on another, and a row-wide label reads as if it
      // were about the users in it. The cell says it, per platform.
      return `<tr><td>${escapeHtml(app)}</td>
        ${PLATFORMS.map((os) => {
          const count = n(os) || `<span class="muted">0</span>`;
          // Marked only where somebody is on it; an empty cell needs no label.
          return pub?.[os] === app && n(os) > 0
            ? `<td class="num" title="The latest ${escapeHtml(osLabel(os))} version"><b class="ok">${count}</b> <span class="badge on">latest</span></td>`
            : `<td class="num">${count}</td>`;
        }).join("")}
        <td class="num"><b>${all}</b></td></tr>`;
    })
    .join("");
  // How far each platform has updated: the share of this month's installs on
  // the version it is offered now, and how many are on 0.2.7 or older — the
  // copies that read the update notice from the old address.
  const onPlatform = (os: string) => vers.filter((v) => v.os === os).reduce((a, v) => a + v.n, 0);
  const onLatest = (os: string) =>
    vers.filter((v) => v.os === os && v.app === pub?.[os]).reduce((a, v) => a + v.n, 0);
  const onOld = (os: string) =>
    vers.filter((v) => v.os === os && /^\d+\.\d+\.\d+$/.test(v.app) && compareVersions(v.app, "0.2.7") >= 0)
      .reduce((a, v) => a + v.n, 0);
  const adoptionRows = `<tr class="total"><td>On latest</td>${PLATFORMS.map(
    (os) => `<td class="num">${pub?.[os] ? ratio(onLatest(os), onPlatform(os)) : `<span class="muted">—</span>`}</td>`,
  ).join("")}<td></td></tr>
    <tr><td>0.2.7 or older</td>${PLATFORMS.map(
      (os) => `<td class="num">${onOld(os) ? `<span class="warn">${onOld(os)}</span>` : `<span class="muted">0</span>`}</td>`,
    ).join("")}<td class="num">${PLATFORMS.reduce((a, os) => a + onOld(os), 0)}</td></tr>`;

  const real = people.filter((x) => !x.mine).length;
  const own = people.length - real;

  const prod = env.PADDLE_ENV === "production";
  // Always Windows, Linux and macOS, zeros included, so every card reads the
  // same way; anything else only when it has a count.
  const chipsFor = <R extends { os: string }>(list: R[], pick: (o: R) => number) => {
    const count = (os: string) => list.filter((o) => o.os === os).reduce((a, o) => a + pick(o), 0);
    const extra = [...new Set(list.map((o) => o.os))].filter(
      (os) => !["windows", "linux", "macos"].includes(os) && count(os) > 0,
    );
    return `<div>${["windows", "linux", "macos", ...extra]
      .map((os) => `<span class="os${count(os) ? "" : " zero"}">${escapeHtml(osLabel(os))} ${count(os)}</span>`)
      .join("")}</div>`;
  };
  const osChips = (pick: (o: OsRow) => number) => chipsFor(byOs, pick);
  const licChips = (pick: (o: LicenceOsRow) => number) => chipsFor(licOs, pick);
  const mineChips = (pick: (o: OsRow) => number) => chipsFor(mineOs, pick);
  const mineTotal = mineOs.reduce((a, o) => a + o.total, 0);
  const mineActive = mineOs.reduce((a, o) => a + o.active, 0);
  const peak = Math.max(1, ...p.series);
  const bars = p.series
    .map((n, i) => {
      const ago = 13 - i;
      const when = ago === 0 ? "today" : `${ago} days ago`;
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
       <a class="pill" href="/admin/export.csv?what=licences">Licences CSV</a>
       <a class="pill" href="/admin/export.csv?what=customers">Customers CSV</a>
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
       <p class="label">Today <span style="text-transform:none;font-weight:400">(Turkey time, vs yesterday)</span></p>
       <div class="grid">
         <div class="stat"><div class="k">New users <span class="muted">(downloaded &amp; opened)</span></div><div class="v">${p.installs_today}</div>
           ${delta(p.installs_today, p.installs_yesterday)}
           ${osChips((o) => o.today)}</div>
         <div class="stat"><div class="k">New paying customers</div><div class="v">${p.subs_today}</div>
           ${delta(p.subs_today, p.subs_yesterday)}
           ${licChips((o) => o.today)}</div>
         <div class="stat"><div class="k">Active users</div><div class="v">${p.active_today}</div>
           <div class="sub">opened the app today</div>
           ${osChips((o) => o.active_today)}</div>
       </div>
     </section>

     <section class="card">
       <p class="label">Overall</p>
       <div class="grid">
         <div class="stat"><div class="k">Live subscribers</div><div class="v">${s.live}</div>
           ${licChips((o) => o.live)}
           <div class="sub">${s.monthly} monthly · ${s.yearly} yearly · ${s.fresh} new in 30d</div></div>
         <div class="stat"><div class="k">Active this week</div><div class="v">${u.weekly}</div>
           ${osChips((o) => o.active)}
           <div class="sub">${u.free_weekly} free · ${u.new_weekly} new this week</div></div>
         <div class="stat"><div class="k">Installs ever seen</div><div class="v">${u.total}</div>
           ${osChips((o) => o.total)}
           <div class="sub">${u.monthly} active this month</div></div>
         <div class="stat${s.expiring > 0 ? " alert" : ""}"><div class="k">Ending in 7 days</div><div class="v">${s.expiring}</div>
           ${licChips((o) => o.expiring)}
           <div class="sub">${s.winding_down} cancelled, still paid</div></div>
         <div class="stat"><div class="k">Your own machines</div><div class="v">${mineTotal}</div>
           ${mineChips((o) => o.total)}
           <div class="sub">${mineActive} active this week · left out of every count above</div></div>
       </div>
       <p class="label" style="margin-top:20px">New installs, last 14 days</p>
       <div class="bars">${bars}</div>
       <div class="bars-axis"><span>14 days ago</span><span>today</span></div>
     </section>

     ${backendsCard(backends)}

     ${revenueCard(money, share)}

     <section class="card">
       <p class="label">Growth &amp; health</p>
       <div class="scroll"><table>
         <tr><th>Platform</th><th class="num">Downloads</th><th class="num">Opened</th>
             <th>Opened after download</th><th>Came back next day</th><th>Still using after a week</th>
             <th class="num">Lost</th></tr>
         ${healthRows}
       </table></div>
       <p class="muted foot">
         <b>Downloads</b>: every release file on GitHub, all versions${dl ? "" : " — GitHub could not be reached just now"}.
         <b>Opened</b>: installs that ever reported in; one person downloading twice counts twice on
         the left and once here. <b>Came back / still using</b>: of the installs old enough to tell,
         how many were seen again at least a day / a week after their first day.
         <b>Lost</b>: used in the last 30 days, but not in the last 7.
       </p>

       <p class="label" style="margin-top:22px">Versions in use, last 30 days${
         pub ? "" : ` <span class="muted">— latest.json could not be read, so no version is marked latest</span>`
       }</p>
       <div class="scroll"><table>
         <tr><th>Version</th>${PLATFORMS.map(
           (os) =>
             `<th class="num">${osLabel(os)}${
               pub?.[os] ? `<br><span class="muted" style="font-weight:400">latest ${escapeHtml(pub[os])}</span>` : ""
             }</th>`,
         ).join("")}<th class="num">Total</th></tr>
         ${versionRows || `<tr><td colspan="5" class="muted">No installs this month.</td></tr>`}
         ${versionRows ? adoptionRows : ""}
       </table></div>

       <p class="label" style="margin-top:22px">Downloads per release</p>
       ${releasesTable(downloadsNow)}
     </section>

     <section class="card" id="users">
       <p class="label">Users, last 30 days — ${real} real${own ? ` · ${own} yours` : ""}</p>
       <p class="muted" style="margin-top:0">One row per install. Press <b>This is me</b> on your
         own machines: they stay in this list, tagged <b>you</b>, but leave every count on this page.</p>
       ${usersTable(people)}
     </section>

     ${webhookCard(hooks)}
     ${failureTable}
     ${endingCard(ending)}

     <details class="card" id="licences"${query ? " open" : ""}>
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
         <p class="muted">Press <b>This is me</b> on your own test purchases: they stay
           listed, tagged <b>you</b>, but are never counted as a customer or a sale.</p>
         ${licenceGroup(rows.filter(isPaddle), true, !!query)}
         ${licenceGroup(rows.filter((r) => !isPaddle(r)), false, !!query)}
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

     ${historyCard(log)}

     <p class="muted foot">
       Refunds and cancellations should normally be done in Paddle — its
       webhook updates this automatically. The Refund button here only marks the
       licence, and does not move any money. Support: ${escapeHtml(supportEmail(env))}
     </p>`,
    `<style>${ADMIN_STYLE}</style>`,
    true,
  );
}
