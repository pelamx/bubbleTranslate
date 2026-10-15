// What the panel draws: the dashboard page and its tables.

import { type Env, USD_AMOUNT, supportEmail } from "../env";
import { DEFAULT_SEATS, isLive } from "../licences";
import { LOGO, escapeHtml, page } from "../pages";
import { now } from "../tokens";
import { type Balance, paddleBalance } from "./balance";
import { COUNTRY_POINTS, LAND_ROWS, MAP_NORTH, MAP_STEP } from "./world";
import { DAY, LOCAL_OFFSET, PAGE_SIZE, date, startOfToday } from "./shared";
import {
  HealthRow,
  LicenceCountryRow,
  LicenceOsRow,
  NOT_MINE_INSTALL,
  NOT_MINE_LICENCE,
  OsRow,
  PLATFORMS,
  Row,
  type AdminLogRow,
  type AdjustmentRow,
  adjustments,
  licencesBySubscription,
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
  countries,
  type CountryRow,
  type SourceRow,
  sources,
  type LimitHits,
  limitHits,
  type Updates,
  updates,
  type BuyFunnel,
  buyFunnel,
  cancelReasons,
  type CancelReasons,
  downloads,
  health,
  ignoreList,
  licenceCountries,
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
  versionCountries,
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
        <td>${u.source === null ? `<span class="muted">—</span>` : u.source === "unmatched" ? `<span class="muted">unmatched</span>` : escapeHtml(u.source)}</td>
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
    <tr><th></th><th>OS</th><th>Version</th><th>Plan</th><th>Source</th><th>First seen</th><th>Last seen</th><th>Id</th><th></th></tr>
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
         SUM(CASE WHEN MAX(last_seen, COALESCE(alive_at, 0)) >= ?1 THEN 1 ELSE 0 END) AS active
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

  /* The frame: a header across the top, the sections listed down the left. */
  body { padding: 0; }
  .sheet.wide { max-width: none; }
  .appbar { position: sticky; top: 0; z-index: 5; display: flex; align-items: center; gap: 12px;
    padding: 8px 24px; background: rgba(15,17,23,.92); backdrop-filter: blur(8px);
    border-bottom: 1px solid var(--border); }
  .appbar .brand { display: flex; align-items: center; gap: 10px; font-size: 20px; font-weight: 700; }
  .appbar .brand svg { width: 30px; height: 30px; }
  .appbar .tag { font-size: 11px; font-weight: 600; letter-spacing: .6px; color: var(--muted);
    background: var(--card-2); border-radius: 6px; padding: 4px 8px; }
  .appbar .spacer { flex: 1; }
  .appbar a.btn, .appbar .env { display: inline-flex; align-items: center; gap: 7px; font-size: 13px;
    color: var(--text); text-decoration: none; border: 1px solid var(--border); border-radius: 9px; padding: 5px 11px; }
  .appbar a.btn:hover { border-color: var(--brand); }
  .appbar .env::before { content: ""; width: 8px; height: 8px; border-radius: 50%; background: var(--faint); }
  .appbar .env.prod { color: var(--accent); border-color: rgba(74,222,128,.45); }
  .appbar .env.prod::before { background: var(--accent); }
  .shell { display: grid; grid-template-columns: 190px minmax(0, 1fr); gap: 22px; padding: 0 24px; }
  .side { position: sticky; top: 52px; align-self: start; padding: 12px 0; display: flex; flex-direction: column; gap: 4px; }
  .side a { display: flex; align-items: center; gap: 10px; color: var(--muted); text-decoration: none;
    font-size: 14px; padding: 8px 12px; border-radius: 10px; border: 1px solid transparent; }
  .side a:hover { color: var(--text); background: var(--card); }
  .side a.on { color: var(--brand); background: rgba(110,168,254,.1); border-color: rgba(110,168,254,.25); }
  .side a .count { margin-left: auto; font-size: 11px; color: var(--muted); }
  .side a .count.bad { color: #0f1117; background: var(--red); border-radius: 999px; padding: 0 7px; }
  .side a .count.warn { color: #0f1117; background: var(--orange); border-radius: 999px; padding: 0 7px; }
  .side .hint { font-size: 11px; color: var(--faint); padding: 14px 12px 0; }
  .side .hint kbd { font-family: inherit; border: 1px solid var(--border); border-radius: 4px; padding: 0 5px; }
  .main { padding: 12px 0 40px; min-width: 0; }
  #overview > .card { padding: 12px 16px; margin-bottom: 12px; }
  #overview .label { margin-bottom: 8px; }
  /* On the entrance the figure sits beside its title, so both rows of
     tiles and the map fit one screen. */
  #overview .stat { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; column-gap: 12px; align-content: start; }
  #overview .stat .head { display: contents; }
  #overview .stat .v { grid-column: 3; grid-row: 1; margin: 0; text-align: right; align-self: start; }
  #overview .stat > .d { grid-column: 2 / -1; margin: 2px 0 0; font-size: 12px; }
  #overview .stat .osbars, #overview .stat .foot-line { grid-column: 1 / -1; }
  #overview .stat .foot-line { margin-top: 8px; align-self: end; }
  #overview .grid.five { grid-template-columns: repeat(auto-fit, minmax(165px, 1fr)); }
  .zone { scroll-margin-top: 76px; }
  .label .aside { text-transform: none; font-weight: 400; letter-spacing: 0; color: var(--faint); }
  details.card, section.card { scroll-margin-top: 76px; }

  /* Icons: drawn with the text colour unless a tone is given. */
  svg.i { width: 18px; height: 18px; flex: none; fill: none; stroke: currentColor; stroke-width: 2;
    stroke-linecap: round; stroke-linejoin: round; vertical-align: -4px; }
  .ico { display: inline-flex; align-items: center; justify-content: center; width: 34px; height: 34px;
    border-radius: 10px; flex: none; }
  .ico svg.i { width: 19px; height: 19px; }
  .t-blue { color: #60a5fa; background: rgba(96,165,250,.14); }
  .t-red { color: #f87171; background: rgba(248,113,113,.14); }
  .t-green { color: #4ade80; background: rgba(74,222,128,.13); }
  .t-teal { color: #2dd4bf; background: rgba(45,212,191,.13); }
  .t-violet { color: #a78bfa; background: rgba(167,139,250,.15); }
  .t-orange { color: #fb923c; background: rgba(251,146,60,.14); }

  /* The stat tiles. */
  .grid { grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); }
  .stat { display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 12px; padding: 11px 14px; }
  .stat .head { display: flex; gap: 12px; align-items: flex-start; }
  .stat .k { margin: 0; font-size: 14px; font-weight: 600; color: var(--text); line-height: 1.3; }
  .stat .k small { display: block; font-size: 12px; font-weight: 400; color: var(--muted); margin-top: 1px; }
  .stat .k .info { color: var(--faint); cursor: help; font-weight: 400; }
  .stat .v { font-size: 30px; font-weight: 800; line-height: 1.15; margin-top: 6px; }
  .stat.mine { border-color: rgba(110,168,254,.55); box-shadow: 0 0 0 1px rgba(110,168,254,.15) inset; }
  .stat.alert { box-shadow: inset 3px 0 0 var(--orange); }
  .osbars { border-top: 1px solid var(--border); margin-top: 8px; padding-top: 7px; display: grid; gap: 3px; }
  .osrow { display: grid; grid-template-columns: 18px 64px 26px 1fr; align-items: center; gap: 8px; font-size: 12.5px; line-height: 1.35; }
  .osrow .nm { color: var(--muted); }
  .osrow b { font-variant-numeric: tabular-nums; }
  .osrow.zero b { color: var(--faint); font-weight: 500; }
  .osrow .ic { display: inline-flex; justify-content: center; font-size: 14px; line-height: 1; }
  .osrow .ic svg { width: 15px; height: 15px; }
  .track { height: 6px; background: var(--bg); border-radius: 99px; overflow: hidden; }
  .track i { display: block; height: 100%; border-radius: 99px; min-width: 0; }
  .os-windows .ic { color: #3b9cff; } .os-windows .track i { background: #3b9cff; }
  .os-linux .track i { background: #f59e0b; }
  .os-macos .ic { color: #e6e8ef; } .os-macos .track i { background: #a855f7; }
  .os-other .track i { background: var(--muted); }
  .stat .foot-line { display: flex; gap: 8px; align-items: flex-start; margin: auto 0 0; padding-top: 7px;
    border-top: 1px solid var(--border); margin-top: 8px; font-size: 12px; line-height: 1.4; color: var(--muted); }
  .stat .foot-line svg.i { width: 15px; height: 15px; margin-top: 2px; }

  /* Today's active users by country, one small card each. */
  .places { margin-top: 10px; background: var(--card-2); border: 1px solid var(--border); border-radius: 12px; padding: 10px 14px; }
  .places .title { display: flex; align-items: center; gap: 10px; font-size: 14px; font-weight: 600; margin: 0 0 6px; }
  .places .title svg { color: var(--brand); }
  .places .title span { font-weight: 400; color: var(--muted); }
  .places .list { display: grid; gap: 8px; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); }
  .place { display: flex; align-items: center; gap: 8px; background: var(--bg); border: 1px solid var(--border);
    border-radius: 9px; padding: 5px 10px; font-size: 13px; }
  .place .flag { font-size: 17px; line-height: 1; }
  .place b { margin-left: auto; font-variant-numeric: tabular-nums; }
  .places .split { display: grid; gap: 18px; align-items: center; grid-template-columns: auto minmax(0, 1fr); }
  svg.world { height: 128px; width: auto; max-width: 100%; display: block; }
  svg.world .land { fill: none; stroke: #2c3a5e; stroke-width: .62; stroke-linecap: round; }
  svg.world .halo { fill: #4f8cff; opacity: .22; animation: halo 2.6s ease-in-out infinite; transform-box: fill-box; transform-origin: center; }
  svg.world .core { fill: #8ec5ff; filter: drop-shadow(0 0 1.2px #4f8cff); }
  svg.world .spot:hover .core { fill: #fff; }
  @keyframes halo { 0%, 100% { transform: scale(.8); opacity: .3; } 50% { transform: scale(1.15); opacity: .12; } }
  @media (prefers-reduced-motion: reduce) { svg.world .halo { animation: none; } }

  /* Below a wide screen the side list folds to its icons, so the tiles keep
     their width; each name is still there on hover. */
  @media (min-width: 901px) and (max-width: 1399px) {
    .shell { grid-template-columns: 52px minmax(0, 1fr); gap: 16px; }
    .side a { justify-content: center; padding: 10px 0; position: relative; }
    .side a .nav-label, .side .hint { display: none; }
    .side a .count { position: absolute; top: 2px; right: 2px; margin: 0; font-size: 10px; }
  }
  @media (max-width: 900px) {
    .shell { grid-template-columns: minmax(0, 1fr); padding: 0 14px; }
    .side { flex-direction: row; overflow-x: auto; top: 58px; z-index: 4; background: var(--bg); padding: 8px 0; }
    .side a { white-space: nowrap; padding: 7px 10px; }
    .side .hint { display: none; }
    .appbar { padding: 10px 14px; flex-wrap: wrap; }
    .appbar a.btn span { display: none; }
    .places .split { grid-template-columns: minmax(0, 1fr); }
    svg.world { width: 100%; height: auto; }
  }
`;

/** Line icons for the panel, drawn inline so the page fetches nothing. */
const ICON_PATHS: Record<string, string> = {
  grid: '<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>',
  chart: '<path d="M3 3v18h18"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/>',
  bars: '<path d="M6 20v-6"/><path d="M12 20V8"/><path d="M18 20V4"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"/>',
  userPlus: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M19 8v6"/><path d="M22 11h-6"/>',
  users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
  cart: '<circle cx="8" cy="21" r="1"/><circle cx="19" cy="21" r="1"/><path d="M2.05 2.05h2l2.66 12.42a2 2 0 0 0 2 1.58h9.78a2 2 0 0 0 1.95-1.57l1.65-7.43H5.12"/>',
  signal: '<path d="M4.9 19.1C1 15.2 1 8.8 4.9 4.9"/><path d="M7.8 16.2c-2.3-2.3-2.3-6.1 0-8.5"/><circle cx="12" cy="12" r="2"/><path d="M16.2 7.8c2.3 2.3 2.3 6.1 0 8.5"/><path d="M19.1 4.9C23 8.8 23 15.1 19.1 19"/>',
  globe: '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
  download: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>',
  clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
  laptop: '<rect width="16" height="11" x="4" y="4" rx="2"/><path d="M2 20h20"/>',
  file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/>',
  card: '<rect width="20" height="14" x="2" y="5" rx="2"/><path d="M2 10h20"/>',
  key: '<circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/>',
  wallet: '<path d="M12 2v20"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/>',
  tools: '<path d="M4 21v-7"/><path d="M4 10V3"/><path d="M12 21v-9"/><path d="M12 8V3"/><path d="M20 21v-5"/><path d="M20 12V3"/><path d="M2 14h4"/><path d="M10 8h4"/><path d="M18 16h4"/>',
  calendar: '<rect width="18" height="18" x="3" y="4" rx="2"/><path d="M16 2v4"/><path d="M8 2v4"/><path d="M3 10h18"/>',
  apple: '<path d="M12 6.5V3a1 1 0 0 1 1-1"/><path d="M18.2 21A15 15 0 0 0 22 11a6 6 0 0 0-10-4.5A6 6 0 0 0 2 11a15 15 0 0 0 3.8 10 3 3 0 0 0 3.6.6 5.5 5.5 0 0 1 5.2 0 3 3 0 0 0 3.6-.6"/>',
};

const icon = (name: string) =>
  `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${ICON_PATHS[name] ?? ""}</svg>`;

/** Windows' four panes, filled rather than outlined, as the logo is. */
const WINDOWS_ICON = `<svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor"><path d="M3 5.5 10.5 4.4v7.1H3zM11.5 4.3 21 3v8.5h-9.5zM3 12.5h7.5v7.1L3 18.5zM11.5 12.5H21V21l-9.5-1.3z"/></svg>`;

function osIcon(os: string): string {
  if (os === "windows") return WINDOWS_ICON;
  if (os === "linux") return "🐧";
  if (os === "macos") return icon("apple");
  return "•";
}

/** One stat tile: an icon, a title, the figure, and whatever goes under it. */
function tile(t: {
  icon: string;
  tone: string;
  title: string;
  hint?: string;
  value: number;
  extra?: string;
  rows: string;
  foot?: string;
  footIcon?: string;
  cls?: string;
}): string {
  return `<div class="stat${t.cls ? ` ${t.cls}` : ""}">
           <div class="head"><span class="ico ${t.tone}">${icon(t.icon)}</span>
             <p class="k">${t.title}${t.hint ? `<small>${t.hint}</small>` : ""}</p></div>
           <div class="v">${t.value}</div>${t.extra ?? ""}
           ${t.rows}
           ${t.foot ? `<p class="foot-line">${icon(t.footIcon ?? "bars")}<span>${t.foot}</span></p>` : ""}
         </div>`;
}

/** Today's active users as one small card per country, largest first. */
function placesPanel(rows: CountryRow[], active: number): string {
  const list = rows.filter((r) => r.today > 0).sort((a, b) => b.today - a.today);
  const body = list.length
    ? `<div class="list">${list
        .map((r) => {
          const { flag, name } = country(r.country);
          const code = r.country === "?" ? "unknown" : r.country;
          return `<div class="place" title="${escapeHtml(name)}"><span class="flag">${flag}</span>${escapeHtml(code)}<b>${r.today}</b></div>`;
        })
        .join("")}</div>`
    : `<p class="muted" style="margin:0">Nobody has opened the app yet today.</p>`;
  return `<div class="places">
       <div class="split">${worldMap(list)}<div>
         <p class="title">${icon("globe")}Active users by location <span>(today's ${active} active user${active === 1 ? "" : "s"})</span></p>
         ${body}</div></div>
     </div>`;
}

/** The land as one path of dots, built once per isolate: it never changes. */
let landPath = "";
function land(): string {
  if (landPath) return landPath;
  const parts: string[] = [];
  LAND_ROWS.forEach((hex, y) => {
    const bits = [...hex].map((c) => parseInt(c, 16).toString(2).padStart(4, "0")).join("");
    for (let x = 0; x < bits.length; x++) if (bits[x] === "1") parts.push(`M${x + 0.5} ${y + 0.5}h0`);
  });
  landPath = parts.join("");
  return landPath;
}

/** The dotted world with a glowing point on each country seen today, sized by
 *  how many. Countries with no known point -- `?`, Tor, unknown -- are left
 *  to the list beside it. */
function worldMap(list: CountryRow[]): string {
  const width = LAND_ROWS[0]!.length * 4;
  const height = LAND_ROWS.length;
  const peak = Math.max(1, ...list.map((r) => r.today));
  const spots = list
    .map((r) => {
      const at = COUNTRY_POINTS[r.country];
      if (!at) return "";
      const x = ((at[0] + 180) / MAP_STEP).toFixed(1);
      const y = ((MAP_NORTH - at[1]) / MAP_STEP + 0.5).toFixed(1);
      const size = 0.8 + Math.sqrt(r.today / peak) * 1.1;
      const { name } = country(r.country);
      return `<g class="spot"><title>${escapeHtml(name)}: ${r.today}</title>
        <circle class="halo" cx="${x}" cy="${y}" r="${(size * 2.6).toFixed(2)}"/>
        <circle class="core" cx="${x}" cy="${y}" r="${size.toFixed(2)}"/></g>`;
    })
    .join("");
  return `<svg class="world" viewBox="0 0 ${width} ${height}" role="img" aria-label="Map of today's active users">
       <path class="land" d="${land()}"/>${spots}
     </svg>`;
}

/** Keeps the side list in step with the page: the section on screen is lit,
 *  and following a link to a folded section unfolds it. `/` jumps to the
 *  licence search. */
const SHELL_SCRIPT = `
  (() => {
    const links = [...document.querySelectorAll('.side a[href^="#"]')];
    const open = (id) => {
      const el = document.getElementById(id);
      if (el && el.tagName === 'DETAILS') el.open = true;
    };
    for (const a of links) a.addEventListener('click', () => open(a.hash.slice(1)));
    if (location.hash) open(location.hash.slice(1));
    const zones = links.map((a) => document.getElementById(a.hash.slice(1))).filter(Boolean);
    const light = () => {
      let current = zones[0];
      for (const z of zones) if (z.getBoundingClientRect().top < 140) current = z;
      if (window.innerHeight + window.scrollY >= document.body.scrollHeight - 4) current = zones[zones.length - 1];
      for (const a of links) a.classList.toggle('on', current && a.hash === '#' + current.id);
    };
    addEventListener('scroll', light, { passive: true });
    light();
    addEventListener('keydown', (e) => {
      if (e.key !== '/' || /INPUT|TEXTAREA|SELECT/.test(document.activeElement?.tagName ?? '')) return;
      e.preventDefault();
      open('licences');
      const q = document.getElementById('q');
      q?.scrollIntoView({ block: 'center' });
      q?.focus();
    });
  })();
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

/** A country code as its flag and English name, or the code alone where the
 *  runtime has no name for it. `?` is installs not seen since it was recorded. */
function countryLabel(code: string): string {
  if (code === "?") return `<span class="muted">not recorded yet</span>`;
  const { flag, name } = country(code);
  return `${flag} ${escapeHtml(name)} <span class="muted">${escapeHtml(code)}</span>`;
}

/** A country code's flag and English name; the name is the code where the
 *  runtime has none (T1 for Tor, XX for unknown -- codes Cloudflare invents). */
function country(code: string): { flag: string; name: string } {
  let name = code;
  try {
    name = new Intl.DisplayNames(["en"], { type: "region" }).of(code) ?? code;
  } catch {
    // No name; the code stands in for it.
  }
  const flag = /^[A-Z]{2}$/.test(code)
    ? String.fromCodePoint(...[...code].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65))
    : "";
  return { flag, name };
}

/** Countries as compact chips -- flag and code, the full name on hover --
 *  largest first. Empty when nobody is counted, so the card stays quiet. */
function countryChips(rows: CountryRow[], pick: (r: CountryRow) => number): string {
  const list = rows.filter((r) => pick(r) > 0).sort((a, b) => pick(b) - pick(a));
  if (!list.length) return "";
  return `<div>${list
    .map((r) => {
      const { flag, name } = country(r.country);
      const code = r.country === "?" ? "unknown" : r.country;
      return `<span class="os" title="${escapeHtml(name)}">${flag} ${escapeHtml(code)} ${pick(r)}</span>`;
    })
    .join("")}</div>`;
}

function sourcesCard(rows: SourceRow[]): string {
  const name = (src: string) =>
    src === "unmatched"
      ? `<span class="muted">Not from a website click</span>`
      : src === "direct"
        ? `direct <span class="muted">— typed in, or no referrer</span>`
        : `<b>${escapeHtml(src)}</b>`;
  const body = rows.length
    ? rows
        .map(
          (r) => `<tr><td>${name(r.src)}</td>
            <td class="num">${r.src === "unmatched" ? `<span class="muted">—</span>` : r.clicks}</td>
            <td class="num"><b>${r.installs}</b></td>
            <td>${ratio(r.active, r.installs)}</td>
            <td class="num">${r.pro || `<span class="muted">0</span>`}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="5" class="muted">No download clicks or new installs in the last 30 days.</td></tr>`;
  return `<section class="card" id="sources">
       <p class="label">Where users come from, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Source</th><th class="num">Download clicks</th><th class="num">Opened</th>
             <th>Still using this week</th><th class="num">Pro now</th></tr>
         ${body}
       </table></div>
       <p class="muted foot">The source is the <code>?utm_source=</code> or <code>?ref=</code> of the
         link that brought someone to the website, or else the site that sent them. Tag every link
         you post — <code>bubbletranslate.app/?ref=reddit</code> — so it gets a row of its own.
         <b>Opened</b> is a best guess: a new install is matched to the latest download click for
         the same system from the same country in the three days before it first opened, so two
         people downloading alike on the same day can swap rows. Recorded from 2 October 2026;
         your own machines are left out of Opened.</p>
     </section>`;
}

function countriesCard(rows: CountryRow[]): string {
  const body = rows.length
    ? rows
        .map(
          (r) => `<tr><td>${countryLabel(r.country)}</td>
            <td class="num"><b>${r.n}</b></td><td class="num">${r.week}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="3" class="muted">No installs this month.</td></tr>`;
  return `<section class="card">
       <p class="label">Installs by country, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Country</th><th class="num">Installs</th><th class="num">Seen this week</th></tr>
         ${body}
       </table></div>
       <p class="muted foot">From the country Cloudflare reads off each ping's address; the address
         itself is not stored. Recorded from 25 September 2026, so an install that has not pinged
         since shows as not recorded yet. Your own machines are left out.</p>
     </section>`;
}

function licenceCountriesCard(rows: LicenceCountryRow[]): string {
  const body = rows.length
    ? rows
        .map(
          (r) => `<tr><td>${countryLabel(r.country)}</td>
            <td class="num"><b>${r.live}</b></td><td class="num">${r.total}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="3" class="muted">No paid licences yet.</td></tr>`;
  return `<section class="card">
       <p class="label">Paid licences by country</p>
       <div class="scroll"><table>
         <tr><th>Country</th><th class="num">Live</th><th class="num">Ever sold</th></tr>
         ${body}
       </table></div>
       <p class="muted foot">From the billing address the buyer gave Paddle. Recorded from
         11 October 2026, so a licence sold earlier shows as not recorded yet until it renews.
         Your own licences are left out.</p>
     </section>`;
}

/** Which version the people using the app right now are on: everyone seen
 *  this week, per version and platform, with today's part of it beside. */
function activeVersionsCard(up: Updates, pub: Record<string, string> | null): string {
  const apps = [...new Set(up.week.map((w) => w.app))].sort(compareVersions).reverse();
  if (apps.length === 0) {
    return `<section class="card">
       <p class="label">Active users by version</p>
       <p class="sub">Nobody has used the app this week.</p>
     </section>`;
  }
  const cell = (n: number, today: number) =>
    n ? `<b>${n}</b> <span class="muted">· ${today} today</span>` : `<span class="muted">0</span>`;
  const pick = (f: (w: Updates["week"][number]) => boolean) => {
    const ws = up.week.filter(f);
    return cell(ws.reduce((a, w) => a + w.n, 0), ws.reduce((a, w) => a + w.today, 0));
  };
  const rows = apps
    .map(
      (app) => `<tr><td>${escapeHtml(app)}</td>
        ${PLATFORMS.map((os) => {
          const c = pick((w) => w.app === app && w.os === os);
          return pub?.[os] === app
            ? `<td class="num">${c} <span class="badge on">latest</span></td>`
            : `<td class="num">${c}</td>`;
        }).join("")}
        <td class="num">${pick((w) => w.app === app)}</td></tr>`,
    )
    .join("");
  return `<section class="card">
       <p class="label">Active users by version</p>
       <div class="scroll"><table>
         <tr><th>Version</th>${PLATFORMS.map((os) => `<th class="num">${escapeHtml(osLabel(os))}</th>`).join("")}
             <th class="num">Total</th></tr>
         ${rows}
         <tr class="total"><td>All</td>${PLATFORMS.map((os) => `<td class="num">${pick((w) => w.os === os)}</td>`).join("")}
             <td class="num">${pick(() => true)}</td></tr>
       </table></div>
       <p class="muted foot">Everyone who used the app in the last 7 days, by the version they had
         then; "today" is the part of them seen since midnight, Turkey time.</p>
     </section>`;
}

/** Whether old copies move to the new version: per platform, who updated in
 *  the last 30 days and who used an older version this week, then each update
 *  as it was seen. */
function updatesCard(up: Updates, pub: Record<string, string> | null): string {
  const latest = (os: string) => pub?.[os];
  const rows = PLATFORMS.map((os) => {
    const mine = up.recent.filter((r) => r.os === os);
    const toLatest = mine.filter((r) => r.to_app === latest(os)).length;
    const week = up.week.filter((w) => w.os === os);
    const used = week.reduce((a, w) => a + w.n, 0);
    const behind = week
      .filter((w) => latest(os) && /^\d+\.\d+\.\d+$/.test(w.app) && compareVersions(w.app, latest(os)!) < 0)
      .reduce((a, w) => a + w.n, 0);
    return `<tr><td>${escapeHtml(osLabel(os))}</td>
      <td>${latest(os) ? escapeHtml(latest(os)!) : `<span class="muted">—</span>`}</td>
      <td class="num"><b>${mine.length}</b></td>
      <td class="num">${latest(os) ? toLatest : `<span class="muted">—</span>`}</td>
      <td>${latest(os) ? (behind ? `<span class="warn">${ratio(behind, used)}</span>` : ratio(0, used)) : `<span class="muted">—</span>`}</td></tr>`;
  }).join("");
  const list = up.recent
    .slice(0, 25)
    .map(
      (r) => `<tr><td><code>${escapeHtml(r.install.slice(0, 8))}</code></td>
        <td>${escapeHtml(osLabel(r.os ?? "unknown"))}</td>
        <td>${escapeHtml(r.from_app)} → <b${r.to_app === latest(r.os ?? "") ? ' class="ok"' : ""}>${escapeHtml(r.to_app)}</b></td>
        <td>${ago(r.at)}</td>
        <td>${r.country ? countryLabel(r.country) : `<span class="muted">—</span>`}</td></tr>`,
    )
    .join("");
  return `<section class="card">
       <p class="label">Updates, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Platform</th><th>Latest</th><th class="num">Updated</th><th class="num">To latest</th>
             <th>Still on an older version, used this week</th></tr>
         ${rows}
       </table></div>
       ${
         list
           ? `<div class="scroll" style="margin-top:14px"><table>
         <tr><th>Install</th><th>Platform</th><th>Version</th><th>Seen</th><th>Country</th></tr>
         ${list}
       </table></div>`
           : `<p class="sub">Nobody has been seen updating yet.</p>`
       }
       <p class="muted foot">An update shows up the first time the updated copy is opened and
         reports in. Recorded from 5 October 2026; anything before that is not here.</p>
     </section>`;
}

function limitCard(hits: LimitHits): string {
  const { base, capped } = hits;
  if (capped.length === 0) {
    return `<section class="card">
       <p class="label">Hitting the free limit</p>
       <p class="sub">Nobody has run out of the free allowance yet — or nobody on a build that says
         so. Copies older than 0.3.7 do not report it.</p>
     </section>`;
  }
  const days = capped.map((r) => Math.max(0, Math.floor((r.first_capped - r.first_seen) / DAY))).sort((a, b) => a - b);
  const median = days[Math.floor((days.length - 1) / 2)];
  const heavy = capped.filter((r) => r.capped_days >= 3).length;
  const pro = capped.filter((r) => r.plan === "pro").length;
  const rows = capped
    .slice(0, 25)
    .map(
      (r) => `<tr><td><code>${escapeHtml(r.install.slice(0, 8))}</code></td>
        <td>${escapeHtml(osLabel(r.os ?? "unknown"))} ${escapeHtml(r.app ?? "")}</td>
        <td>${r.country ? countryLabel(r.country) : `<span class="muted">—</span>`}</td>
        <td class="num">${Math.max(0, Math.floor((r.first_capped - r.first_seen) / DAY))}</td>
        <td class="num"><b>${r.capped_days}</b></td>
        <td>${ago(r.last_capped)}</td>
        <td>${r.plan === "pro" ? `<span class="badge on">pro now</span>` : `<span class="muted">free</span>`}</td></tr>`,
    )
    .join("");
  return `<section class="card">
       <p class="label">Hitting the free limit</p>
       <div class="scroll"><table>
         <tr><th>Ran out at least once</th><th>Days until the first time</th>
             <th>On 3 or more days</th><th>Of those who ran out, on Pro now</th></tr>
         <tr><td>${ratio(capped.length, base)}</td>
             <td class="num">median <b>${median}</b></td>
             <td class="num"><b>${heavy}</b></td>
             <td>${ratio(pro, capped.length)}</td></tr>
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Install</th><th>Version</th><th>Country</th><th class="num">Days to first</th>
             <th class="num">Days capped</th><th>Last</th><th>Plan</th></tr>
         ${rows}
       </table></div>
       <p class="muted foot">Reported by the app the first time on a day that the ten free
         translations run out; copies older than 0.3.7 do not send it. A free install that keeps
         running out is someone the app is useful to who has not bought — the people pricing is for.
         Your own machines are left out.</p>
     </section>`;
}

const SRC_LABEL: Record<string, string> = {
  bubble: "App bubble, out of free translations",
  "bubble-low": "App bubble, three or fewer left",
  window: "App window",
  unknown: "Not recorded (orders before 2026-10-15)",
  direct: "No source (website, typed, bookmarked)",
};

function funnelCard(f: BuyFunnel): string {
  if (f.visits === 0 && f.started === 0) {
    return `<section class="card">
       <p class="label">Buy page, last 30 days</p>
       <p class="sub">Nobody has opened the buy page yet since visits started being counted.</p>
     </section>`;
  }
  const srcRows = f.bySrc
    .map(
      (r) => `<tr><td>${escapeHtml(SRC_LABEL[r.src] ?? r.src)}</td><td class="num"><b>${r.n}</b></td>
        <td class="num">${r.started}</td><td class="num">${r.paid ? `<b>${r.paid}</b>` : `<span class="muted">0</span>`}</td></tr>`,
    )
    .join("");
  const langs = f.byLang.map((r) => `${escapeHtml(r.lang.toUpperCase())} ${r.n}`).join(" · ");
  return `<section class="card">
       <p class="label">Buy page, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Opened the page</th><th>Pressed “Continue to payment”</th><th>Paid</th></tr>
         <tr><td class="num"><b>${f.visits}</b></td>
             <td>${ratio(f.started, f.visits)}</td>
             <td>${ratio(f.paid, f.started)}</td></tr>
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Came from</th><th class="num">Visits</th><th class="num">Started</th><th class="num">Paid</th></tr>
         ${srcRows}
       </table></div>
       <p class="muted foot">Shown in: ${langs || "—"}. Every page load counts, including yours and
         any crawler's, so a handful of visits with no payment is not yet a verdict.</p>
     </section>`;
}

const CANCEL_LABEL: Record<string, string> = {
  price: "Costs too much",
  rarely: "Doesn't use it often enough",
  "free-enough": "Free translations are enough",
  quality: "Translations not good enough",
  broken: "Something didn't work",
  "other-tool": "Using something else",
  other: "Another reason",
};

function cancelReasonsCard(c: CancelReasons): string {
  const total = c.byReason.reduce((a, r) => a + r.n, 0);
  if (total === 0) {
    return `<section class="card">
       <p class="label">Why people cancel, last 90 days</p>
       <p class="sub">No answers yet. The account page's cancel form asks; Paddle's own portal does not.</p>
     </section>`;
  }
  const rows = c.byReason
    .map(
      (r) => `<tr><td>${escapeHtml(CANCEL_LABEL[r.reason] ?? r.reason)}</td><td class="num"><b>${r.n}</b></td></tr>`,
    )
    .join("");
  const notes = c.notes
    .map(
      (n) => `<tr><td>${escapeHtml(new Date(n.at * 1000).toISOString().slice(0, 10))}</td>
        <td>${escapeHtml(CANCEL_LABEL[n.reason] ?? n.reason)}${n.cycle ? ` · ${escapeHtml(n.cycle)}` : ""}</td>
        <td>${escapeHtml(n.note)}</td></tr>`,
    )
    .join("");
  return `<section class="card">
       <p class="label">Why people cancel, last 90 days — ${total} answered</p>
       <div class="scroll"><table>
         <tr><th>Reason</th><th class="num">Answers</th></tr>
         ${rows}
       </table></div>
       ${notes ? `<div class="scroll" style="margin-top:14px"><table>
         <tr><th>When</th><th>Reason</th><th>In their words</th></tr>
         ${notes}
       </table></div>` : ""}
       <p class="muted foot">Asked on the account page's cancel form, and optional. A cancellation
         made in Paddle's own portal is not asked, so this is a sample, not every one.</p>
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

/** Minor units as "$2.00" for USD, "2.00 EUR" otherwise. */
function minorAmount(minor: number | null, currency: string | null): string {
  if (minor === null) return "—";
  const v = (minor / 100).toFixed(2);
  return !currency || currency === "USD" ? `$${v}` : `${v} ${currency}`;
}

/** Refunds and chargebacks from Paddle: what went back to each buyer, what it
 *  took out of the balance, and where Paddle's approval stands. Open, and
 *  amber, while any refund is still waiting for Paddle to approve it. */
export function refundsCard(list: AdjustmentRow[]): string {
  const cls: Record<string, string> = { approved: "ok", pending_approval: "warn", rejected: "err" };
  const takesMoney = (a: AdjustmentRow) =>
    a.status === "approved" && (a.action === "refund" || a.action === "chargeback");
  const pending = list.filter((a) => a.status === "pending_approval").length;
  // Totals of what was actually approved, per currency: a refund pending or
  // rejected has not moved anything yet.
  const sums = new Map<string, { back: number; balance: number; fee: number }>();
  for (const a of list.filter(takesMoney)) {
    const key = a.balance_currency ?? a.currency ?? "USD";
    const s = sums.get(key) ?? { back: 0, balance: 0, fee: 0 };
    if (a.currency === key) s.back += a.total ?? 0;
    s.balance += (a.earnings ?? 0) + (a.retained_fee ?? 0);
    s.fee += a.retained_fee ?? 0;
    sums.set(key, s);
  }
  const body = list
    .map((a) => {
      const balance = (a.earnings ?? 0) + (a.retained_fee ?? 0);
      const when = a.paddle_created ?? a.updated_at;
      return `<tr${a.mine ? ' class="mine"' : ""}>
        <td title="${escapeHtml(new Date(when * 1000).toISOString())}">${escapeHtml(date(when))} <span class="muted">${ago(when)}</span></td>
        <td>${escapeHtml(a.action)}${a.type ? ` <span class="muted">${escapeHtml(a.type)}</span>` : ""}</td>
        <td class="${cls[a.status] ?? "muted"}">${escapeHtml(a.status.replace("_", " "))}</td>
        <td>${
          a.licence_id
            ? `<a href="/admin?q=${encodeURIComponent(a.licence_id)}#licences"><code>${escapeHtml(a.licence_id)}</code></a>${
                a.mine ? ` <span class="badge you">you</span>` : ""
              }`
            : "—"
        }<div class="muted">${escapeHtml(a.email ?? "")}</div></td>
        <td class="num">${minorAmount(a.total, a.currency)}${
          a.tax ? `<div class="muted">tax ${minorAmount(a.tax, a.currency)}</div>` : ""
        }</td>
        <td class="num">${
          a.earnings === null ? "—" : `−${minorAmount(balance, a.balance_currency)}`
        }${a.retained_fee ? `<div class="muted">fee kept ${minorAmount(a.retained_fee, a.balance_currency)}</div>` : ""}</td>
        <td><code>${escapeHtml(a.adjustment_id)}</code>${
          a.transaction_id ? `<div class="muted"><code>${escapeHtml(a.transaction_id)}</code></div>` : ""
        }${a.reason ? `<div class="muted">${escapeHtml(a.reason)}</div>` : ""}</td></tr>`;
    })
    .join("");
  const summary = [...sums]
    .map(
      ([cur, s]) =>
        `${minorAmount(s.back, cur)} back to buyers · ${minorAmount(s.balance, cur)} out of the balance, ${minorAmount(s.fee, cur)} of it Paddle's kept fees`,
    )
    .join("<br>");
  return `<details class="card${pending ? " warnbox" : ""}" id="refunds"${pending ? " open" : ""}>
    <summary>Refunds (${list.length}${pending ? ` · ${pending} waiting for Paddle` : ""})</summary>
    <div class="body">${
      list.length
        ? `${summary ? `<p style="margin-top:0"><b>Approved:</b> ${summary}</p>` : ""}
           <div class="scroll"><table><tr><th>Created</th><th>Kind</th><th>Status</th><th>Licence</th>
             <th class="num">To buyer</th><th class="num">From balance</th><th>Paddle ids</th></tr>${body}</table></div>
           <p class="muted foot"><b>To buyer</b> is the whole refund, tax included, back on their card.
             <b>From balance</b> is your earnings on the sale plus the fee Paddle keeps on a refund.
             <b>Pending approval</b> means Paddle has not reviewed it yet; nothing has moved.
             <b>Partial</b> only means it was made per item — check the amount.</p>`
        : `<p class="muted">No refunds or chargebacks yet. They appear here as Paddle sends them.</p>`
    }</div>
  </details>`;
}

/** The Paddle balance, sale by sale: what each buyer paid, the tax and fee
 *  Paddle took, what was earned, what a refund took back, and the running
 *  balance. Paddle's own figures, read live. */
export function balanceCard(
  b: Balance | string,
  licences: Map<string, { id: string; mine: boolean }>,
): string {
  if (typeof b === "string") {
    return `<details class="card" id="balance"><summary>Paddle balance</summary>
      <div class="body"><p class="muted">${escapeHtml(b)}</p></div></details>`;
  }
  const m = (v: number, c = b.currency) => minorAmount(v, c);
  const neg = (v: number, c = b.currency) => (v ? `−${m(v, c)}` : `<span class="muted">—</span>`);
  const body = b.rows
    .map((r) => {
      const lic = r.subscription_id ? licences.get(r.subscription_id) : undefined;
      const kind = r.origin === "subscription_recurring" ? "renewal" : r.origin === "web" ? "new" : (r.origin ?? "—");
      const taken = r.refunded + r.retained_fee + r.chargeback_fee;
      return `<tr${lic?.mine ? ' class="mine"' : ""}>
        <td title="${escapeHtml(new Date(r.at * 1000).toISOString())}">${escapeHtml(date(r.at))}</td>
        <td>${escapeHtml(r.email ?? "—")}${lic?.mine ? ` <span class="badge you">you</span>` : ""}<div class="muted">${
          lic ? `<a href="/admin?q=${encodeURIComponent(lic.id)}#licences"><code>${escapeHtml(lic.id)}</code></a> · ` : ""
        }<code>${escapeHtml(r.transaction_id)}</code></div></td>
        <td>${escapeHtml(kind)}${r.country ? ` <span class="muted">${escapeHtml(r.country)}</span>` : ""}</td>
        <td class="num">${m(r.paid, r.currency)}</td>
        <td class="num">${neg(r.tax, r.currency)}</td>
        <td class="num">${neg(r.fee, r.balance_currency)}</td>
        <td class="num">${m(r.earnings, r.balance_currency)}</td>
        <td class="num">${neg(taken, r.balance_currency)}${
          r.retained_fee ? `<div class="muted">fee kept ${m(r.retained_fee, r.balance_currency)}</div>` : ""
        }${r.chargeback_fee ? `<div class="muted">chargeback fee ${m(r.chargeback_fee, r.balance_currency)}</div>` : ""}</td>
        <td class="num"><b>${m(r.net, r.balance_currency)}</b></td></tr>`;
    })
    .join("");
  const taken = b.refunded + b.retained_fee + b.chargeback_fee;
  const total = `<tr><td colspan="3"><b>Total</b> <span class="muted">${b.rows.length} sales</span></td>
    <td class="num"><b>${m(b.paid)}</b></td><td class="num">${neg(b.tax)}</td><td class="num">${neg(b.fee)}</td>
    <td class="num"><b>${m(b.earnings)}</b></td><td class="num">${neg(taken)}</td>
    <td class="num"><b>${m(b.balance)}</b></td></tr>`;
  return `<details class="card" id="balance">
    <summary>Paddle balance — ${m(b.balance)}</summary>
    <div class="body">${
      b.rows.length
        ? `<p style="margin-top:0">Earned <b>${m(b.earnings)}</b> from ${m(b.paid)} paid ·
             Paddle fees <b>${m(b.fee)}</b>${b.retained_fee ? ` (+${m(b.retained_fee)} kept on refunds)` : ""} ·
             tax ${m(b.tax)} · refunds took ${m(taken)} · balance <b>${m(b.balance)}</b></p>
           ${b.truncated ? `<p class="warn">Only the first 600 transactions were read; the totals are incomplete.</p>` : ""}
           ${b.note ? `<p class="warn">${escapeHtml(b.note)}</p>` : ""}
           <div class="scroll"><table><tr><th>Date</th><th>Buyer</th><th>Kind</th><th class="num">Paid</th>
             <th class="num">Tax</th><th class="num">Paddle fee</th><th class="num">Earned</th>
             <th class="num">Refunded</th><th class="num">Net</th></tr>${body}${total}</table></div>
           <p class="muted foot">Paddle's own figures, read live. <b>Paid</b> is what the buyer was charged, tax included.
             <b>Earned</b> is what is left after tax and Paddle's fee (5% + 50¢). <b>Refunded</b> is those earnings
             taken back plus the fee Paddle keeps. <b>Net</b> is what the sale leaves in the balance, and the
             total of Net is the balance. It is before payouts — Paddle's API does not list
             them, so after the first payout it will read higher than the dashboard by what was paid out.</p>`
        : `<p class="muted">No completed sales yet.</p>`
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
  const [s, u, p, byOs, mineOs, licOs, rows, failures, people, hl, vers, downloadsNow, pub, log, hooks, ending, money, share, backends, byCountry, hits, funnel, bySource, ups, versCountry, licCountry, refunds, balance, licBySub, whyCancel] = await Promise.all([
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
    countries(env),
    limitHits(env),
    buyFunnel(env),
    sources(env),
    updates(env),
    versionCountries(env),
    licenceCountries(env),
    adjustments(env),
    paddleBalance(env),
    licencesBySubscription(env),
    cancelReasons(env),
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
  //
  // Every version anyone runs gets a row, the operator's own machines
  // included, and so does every version a platform is offered now: a release
  // shows up here the moment it is published and fills in as copies update,
  // rather than only once a real user has reached it.
  const versionList = [
    ...new Set([...vers.map((v) => v.app), ...Object.values(pub ?? {})]),
  ].sort(compareVersions);
  const versionRows = versionList
    .map((app) => {
      const n = (os: string) => vers.filter((v) => v.app === app && v.os === os).reduce((a, v) => a + v.n, 0);
      const mineOn = (os: string) =>
        vers.filter((v) => v.app === app && v.os === os).reduce((a, v) => a + v.mine, 0);
      const all = vers.filter((v) => v.app === app).reduce((a, v) => a + v.n, 0);
      // Your own machines are named beside the count, never added to it.
      const yours = (os: string) =>
        mineOn(os) ? ` <span class="muted" title="Your own machines, left out of the count">+${mineOn(os)} you</span>` : "";
      // No "latest" on the row itself: a version can be current on one
      // platform and behind on another, and a row-wide label reads as if it
      // were about the users in it. The cell says it, per platform.
      return `<tr><td>${escapeHtml(app)}</td>
        ${PLATFORMS.map((os) => {
          const count = n(os) || `<span class="muted">0</span>`;
          // Marked wherever it is the version offered now, even before anyone
          // has updated to it: that is the row everyone is expected to reach.
          return pub?.[os] === app
            ? `<td class="num" title="The latest ${escapeHtml(osLabel(os))} version"><b class="ok">${count}</b> <span class="badge on">latest</span>${yours(os)}</td>`
            : `<td class="num">${count}${yours(os)}</td>`;
        }).join("")}
        <td class="num"><b>${all}</b></td>
        <td>${countryChips(
          versCountry.filter((c) => c.app === app).map((c) => ({ country: c.country, n: c.n, week: 0, today: 0 })),
          (c) => c.n,
        )}</td></tr>`;
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
  ).join("")}<td></td><td></td></tr>
    <tr><td>0.2.7 or older</td>${PLATFORMS.map(
      (os) => `<td class="num">${onOld(os) ? `<span class="warn">${onOld(os)}</span>` : `<span class="muted">0</span>`}</td>`,
    ).join("")}<td class="num">${PLATFORMS.reduce((a, os) => a + onOld(os), 0)}</td><td></td></tr>`;

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
    const oses = ["windows", "linux", "macos", ...extra];
    // Each bar is that platform's share of the tile's figure.
    const most = Math.max(1, oses.reduce((a, os) => a + count(os), 0));
    return `<div class="osbars">${oses
      .map((os) => {
        const n = count(os);
        const kind = ["windows", "linux", "macos"].includes(os) ? os : "other";
        return `<div class="osrow os-${kind}${n ? "" : " zero"}"><span class="ic">${osIcon(os)}</span><span class="nm">${escapeHtml(osLabel(os))}</span> <b>${n}</b><span class="track"><i style="width:${Math.round((n / most) * 100)}%"></i></span></div>`;
      })
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

  const online = byOs.reduce((a, o) => a + (o.online ?? 0), 0);
  const today = new Date((startOfToday() + LOCAL_OFFSET) * 1000).toLocaleDateString("en-GB", {
    weekday: "long", day: "numeric", month: "long", timeZone: "UTC",
  });
  const navCount = (n: number, tone = "") => (n > 0 ? `<span class="count${tone ? ` ${tone}` : ""}">${n}</span>` : "");
  const nav = [
    ["overview", "grid", "Overview", ""],
    ["analytics", "chart", "Analytics", ""],
    ["users", "user", "Users", navCount(real)],
    ["payments", "card", "Payments", hooks.bad ? navCount(hooks.bad, "bad") : navCount(ending.length, ending.length ? "warn" : "")],
    ["licences", "key", "Licences", ""],
    ["tools", "tools", "Tools", ""],
    ["revenue", "wallet", "Revenue", ""],
  ]
    .map(([id, ic, label, extra]) => `<a href="#${id}" title="${label}">${icon(ic)}<span class="nav-label">${label}</span>${extra}</a>`)
    .join("");

  return page(
    "Admin — bubbleTranslate",
    `<header class="appbar">
       <span class="brand">${LOGO}bubbleTranslate</span><span class="tag">ADMIN</span>
       <span class="spacer"></span>
       <a class="btn" href="/admin/export.csv?what=licences" title="Licences CSV">${icon("file")}<span>Licences CSV</span></a>
       <a class="btn" href="/admin/export.csv?what=customers" title="Customers CSV">${icon("file")}<span>Customers CSV</span></a>
       <span class="env${prod ? " prod" : ""}">${escapeHtml(env.PADDLE_ENV ?? "sandbox")}</span>
     </header>
     <div class="shell">
     <nav class="side" aria-label="Sections">
       ${nav}
       <p class="hint">Press <kbd>/</kbd> to search licences</p>
     </nav>
     <main class="main">
     ${notice.message ? `<p class="ok">${escapeHtml(notice.message)}</p>` : ""}
     ${notice.error ? `<p class="err">${escapeHtml(notice.error)}</p>` : ""}
     ${
       notice.key
         ? `<p class="warn">Give this to the customer. It is shown once and cannot be
             looked up again:</p><div class="key">${escapeHtml(notice.key)}</div>`
         : ""
     }

     <div class="zone" id="overview">
     <section class="card">
       <p class="label">Today <span class="aside">· ${escapeHtml(today)} (Turkey time, vs yesterday)</span></p>
       <div class="grid">
         ${tile({ icon: "userPlus", tone: "t-blue", title: "New users", hint: "Downloaded &amp; opened",
           value: p.installs_today, extra: delta(p.installs_today, p.installs_yesterday), rows: osChips((o) => o.today) })}
         ${tile({ icon: "cart", tone: "t-red", title: "New paying customers",
           value: p.subs_today, extra: delta(p.subs_today, p.subs_yesterday), rows: licChips((o) => o.today) })}
         ${tile({ icon: "signal", tone: "t-green", title: "Online now", hint: "App open in the last 10 minutes",
           value: online, rows: osChips((o) => o.online ?? 0) })}
         ${tile({ icon: "users", tone: "t-teal", title: "Active users", hint: "Opened the app today",
           value: p.active_today, rows: osChips((o) => o.active_today) })}
       </div>
       ${placesPanel(byCountry, p.active_today)}
     </section>

     <section class="card">
       <p class="label">Overall</p>
       <div class="grid five">
         ${tile({ icon: "users", tone: "t-green", title: "Live subscribers", value: s.live,
           rows: licChips((o) => o.live), footIcon: "calendar",
           foot: `${s.monthly} monthly · ${s.yearly} yearly · ${s.fresh} new in 30d` })}
         ${tile({ icon: "bars", tone: "t-violet", title: "Active this week", value: u.weekly,
           rows: osChips((o) => o.active), footIcon: "users",
           foot: `${u.free_weekly} free · ${u.new_weekly} new this week` })}
         ${tile({ icon: "download", tone: "t-blue", title: "Installs ever seen", value: u.total,
           rows: osChips((o) => o.total), footIcon: "chart", foot: `${u.monthly} active this month` })}
         ${tile({ icon: "clock", tone: "t-orange", title: "Ending in 7 days", value: s.expiring,
           cls: s.expiring > 0 ? "alert" : "", rows: licChips((o) => o.expiring), footIcon: "card",
           foot: `${s.winding_down} cancelled, still paid` })}
         ${tile({ icon: "laptop", tone: "t-blue", cls: "mine",
           title: `Your own machines <span class="info" title="Machines you marked as yours. They are left out of every other count on this page.">ⓘ</span>`,
           value: mineTotal, rows: mineChips((o) => o.total), footIcon: "laptop",
           foot: `${mineActive} active this week · excluded from counts above` })}
       </div>
       <p class="label" style="margin-top:20px">New installs, last 14 days</p>
       <div class="bars">${bars}</div>
       <div class="bars-axis"><span>14 days ago</span><span>today</span></div>
     </section>
     </div>

     <div class="zone" id="analytics">
     ${backendsCard(backends)}

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
         ).join("")}<th class="num">Total</th><th>From</th></tr>
         ${versionRows || `<tr><td colspan="6" class="muted">No installs this month.</td></tr>`}
         ${versionRows ? adoptionRows : ""}
       </table></div>

       <p class="label" style="margin-top:22px">Downloads per release</p>
       ${releasesTable(downloadsNow)}
     </section>

     ${activeVersionsCard(ups, pub)}

     ${updatesCard(ups, pub)}

     ${limitCard(hits)}

     ${funnelCard(funnel)}

     ${sourcesCard(bySource)}

     ${countriesCard(byCountry)}
     </div>

     ${licenceCountriesCard(licCountry)}

     <details class="card" id="users" open>
       <summary>Users, last 30 days — ${real} real${own ? ` · ${own} yours` : ""}</summary>
       <div class="body">
         <p class="muted" style="margin-top:0">One row per install. Press <b>This is me</b> on your
           own machines: they stay in this list, tagged <b>you</b>, but leave every count on this page.</p>
         ${usersTable(people)}
       </div>
     </details>

     <div class="zone" id="payments">
     ${balanceCard(balance, licBySub)}
     ${refundsCard(refunds)}
     ${cancelReasonsCard(whyCancel)}
     ${webhookCard(hooks)}
     ${failureTable}
     ${endingCard(ending)}
     </div>

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

     <div class="zone" id="tools">
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
     </div>

     ${revenueCard(money, share)}

     <p class="muted foot">
       Refunds and cancellations should normally be done in Paddle — its
       webhook updates this automatically. The Refund button here only marks the
       licence, and does not move any money. Support: ${escapeHtml(supportEmail(env))}
     </p>
     </main>
     </div>
     <script>${SHELL_SCRIPT}</script>`,
    `<style>${ADMIN_STYLE}</style>`,
    true,
  );
}
