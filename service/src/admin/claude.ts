// What Claude costs, from both ends.
//
// The app reads the screen through Claude Haiku on the user's own Anthropic
// key, straight to Anthropic, so the service never sees those requests. Two
// figures stand in for it:
//
// - What the app reports. The daily ping carries each install's requests and
//   tokens per day and model (`claude_usage`); priced here at list price, so
//   it is an estimate of what users are billed, not a bill.
// - What your own Anthropic organisation was billed, read live from the Usage
//   and Cost Admin API with `ANTHROPIC_ADMIN_KEY`. That is your key only --
//   development, testing -- and none of your users'.

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { DAY } from "./shared";
import { NOT_MINE_INSTALL, mineInstalls } from "./queries";

/** List prices in dollars per million tokens, for prompts under 100K tokens
 *  (every request the app makes). Cache reads at a tenth of input and cache
 *  writes at 1.25x are Anthropic's usual multipliers; the app sets no cache
 *  breakpoints, so both stay near zero. A model not listed is shown without
 *  a cost rather than with a wrong one. */
export const CLAUDE_PRICES: Record<string, { input: number; output: number }> = {
  "claude-haiku-5-5": { input: 0.1, output: 0.5 },
  "claude-sonnet-5": { input: 2, output: 10 },
};

export interface Tokens {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}

/** Dollars at list price, or null for a model without one. */
export function listCost(model: string, t: Tokens): number | null {
  const p = CLAUDE_PRICES[model];
  if (!p) return null;
  return (t.input * p.input + t.cache_read * p.input * 0.1 + t.cache_write * p.input * 1.25 + t.output * p.output) / 1e6;
}

export interface ReportedDay extends Tokens {
  day: string;
  model: string;
  installs: number;
  requests: number;
  failed: number;
}

export interface Reported {
  days: ReportedDay[];
  /** Distinct installs over the whole window, per model. */
  installs: Record<string, number>;
}

/** The last 30 days of what installs reported, the operator's own machines
 *  left out. Days are the installs' own local dates. */
export async function claudeReported(env: Env): Promise<Reported> {
  // A panel that will not open is worse than a card that is empty: the
  // table is new, and a database that has not had the schema applied yet
  // must not take the rest of the page down with it.
  try {
    return await readReported(env);
  } catch (err) {
    console.error("could not read claude_usage", err);
    return { days: [], installs: {} };
  }
}

async function readReported(env: Env): Promise<Reported> {
  const since = new Date((Math.floor(Date.now() / 1000) - 29 * DAY) * 1000).toISOString().slice(0, 10);
  const mine = await mineInstalls(env);
  const [days, totals] = await Promise.all([
    env.DB.prepare(
      `SELECT day, model, COUNT(DISTINCT install) AS installs,
              SUM(requests) AS requests, SUM(failed) AS failed,
              SUM(input) AS input, SUM(output) AS output,
              SUM(cache_read) AS cache_read, SUM(cache_write) AS cache_write
         FROM claude_usage
        WHERE day >= ?1 AND ${NOT_MINE_INSTALL}
        GROUP BY day, model
        ORDER BY day DESC, model`,
    )
      .bind(since, null, null, null, null, null, null, null, mine)
      .all<ReportedDay>(),
    env.DB.prepare(
      `SELECT model, COUNT(DISTINCT install) AS installs
         FROM claude_usage
        WHERE day >= ?1 AND ${NOT_MINE_INSTALL}
        GROUP BY model`,
    )
      .bind(since, null, null, null, null, null, null, null, mine)
      .all<{ model: string; installs: number }>(),
  ]);
  return {
    days: days.results ?? [],
    installs: Object.fromEntries((totals.results ?? []).map((r) => [r.model, r.installs])),
  };
}

// -- your own Anthropic bill -----------------------------------------------

export interface AccountModel extends Tokens {
  model: string;
  /** Dollars, from the cost report. */
  cost: number;
}

export interface Account {
  /** Dollars per UTC day, oldest first. */
  days: { day: string; cost: number }[];
  models: AccountModel[];
  total: number;
  today: number;
}

const ANTHROPIC_API = "https://api.anthropic.com/v1/organizations";

/** Every page of one report, up to a few: a 30-day window at a day per
 *  bucket fits in one, and the cap only stops a runaway cursor. */
async function report(env: Env, path: string, params: [string, string][]): Promise<any[] | string> {
  const buckets: any[] = [];
  let page: string | null = null;
  for (let i = 0; i < 4; i++) {
    const url = new URL(`${ANTHROPIC_API}/${path}`);
    for (const [k, v] of params) url.searchParams.append(k, v);
    if (page) url.searchParams.set("page", page);
    const response = await fetch(url, {
      headers: {
        "x-api-key": env.ANTHROPIC_ADMIN_KEY ?? "",
        "anthropic-version": "2023-06-01",
        "user-agent": "bubbleTranslate-admin/1 (https://bubbletranslate.app)",
      },
      signal: AbortSignal.timeout(8000),
    });
    if (!response.ok) {
      const body: any = await response.json().catch(() => null);
      const detail = body?.error?.message ? `: ${String(body.error.message)}` : ".";
      return `Anthropic answered ${response.status}${detail}`;
    }
    const body: any = await response.json();
    if (Array.isArray(body?.data)) buckets.push(...body.data);
    if (!body?.has_more || !body?.next_page) return buckets;
    page = String(body.next_page);
  }
  return buckets;
}

/** Your organisation's last 30 days, read live. A string is the reason it
 *  could not be read, for the card to show. */
export async function anthropicAccount(env: Env): Promise<Account | string> {
  if (!env.ANTHROPIC_ADMIN_KEY) return "No Anthropic Admin API key is configured.";
  const start = Math.floor(Date.now() / 1000 / DAY) * DAY - 29 * DAY;
  const window: [string, string][] = [
    ["starting_at", new Date(start * 1000).toISOString().replace(".000", "")],
    ["bucket_width", "1d"],
    ["limit", "31"],
  ];
  try {
    const [costs, usage] = await Promise.all([
      report(env, "cost_report", [...window, ["group_by[]", "description"]]),
      report(env, "usage_report/messages", [...window, ["group_by[]", "model"]]),
    ]);
    if (typeof costs === "string") return costs;
    if (typeof usage === "string") return usage;
    return summariseAccount(costs, usage);
  } catch (err) {
    console.error("could not read the Anthropic usage", err);
    return "Anthropic could not be reached just now.";
  }
}

/** Dollars your organisation was billed from the start of `at`'s UTC day to
 *  now, from the Cost API. The whole of that first day is counted, so a
 *  figure entered mid-day errs towards less left rather than more. A string
 *  is why it could not be read. */
export async function anthropicSpentSince(env: Env, at: number): Promise<number | string> {
  if (!env.ANTHROPIC_ADMIN_KEY) return "No Anthropic Admin API key is configured.";
  const start = Math.floor(at / DAY) * DAY;
  try {
    const costs = await report(env, "cost_report", [
      ["starting_at", new Date(start * 1000).toISOString().replace(".000", "")],
      ["bucket_width", "1d"],
      ["limit", "31"],
    ]);
    if (typeof costs === "string") return costs;
    let total = 0;
    for (const b of costs) {
      for (const r of Array.isArray(b?.results) ? b.results : []) total += num(r?.amount) / 100;
    }
    return total;
  } catch (err) {
    console.error("could not read the Anthropic cost", err);
    return "Anthropic could not be reached just now.";
  }
}

const num = (v: unknown) => {
  const n = Number(v);
  return Number.isFinite(n) ? n : 0;
};

/** The two reports' buckets as one summary. Cost amounts are cents as
 *  decimal strings; anything that is not a model's tokens (web search, code
 *  execution) is counted in the total under its own description. */
export function summariseAccount(costBuckets: any[], usageBuckets: any[]): Account {
  const byModel = new Map<string, AccountModel>();
  const model = (name: string) => {
    let m = byModel.get(name);
    if (!m) {
      m = { model: name, cost: 0, input: 0, output: 0, cache_read: 0, cache_write: 0 };
      byModel.set(name, m);
    }
    return m;
  };
  const days: { day: string; cost: number }[] = [];
  for (const b of costBuckets) {
    let cost = 0;
    for (const r of Array.isArray(b?.results) ? b.results : []) {
      const dollars = num(r?.amount) / 100;
      cost += dollars;
      model(r?.model ? String(r.model) : String(r?.description ?? "other")).cost += dollars;
    }
    days.push({ day: String(b?.starting_at ?? "").slice(0, 10), cost });
  }
  for (const b of usageBuckets) {
    for (const r of Array.isArray(b?.results) ? b.results : []) {
      const m = model(r?.model ? String(r.model) : "other");
      m.input += num(r?.uncached_input_tokens);
      m.output += num(r?.output_tokens);
      m.cache_read += num(r?.cache_read_input_tokens);
      m.cache_write +=
        num(r?.cache_creation?.ephemeral_5m_input_tokens) + num(r?.cache_creation?.ephemeral_1h_input_tokens);
    }
  }
  days.sort((a, b) => a.day.localeCompare(b.day));
  const today = new Date().toISOString().slice(0, 10);
  return {
    days,
    models: [...byModel.values()]
      .filter((m) => m.cost > 0 || m.input + m.output + m.cache_read + m.cache_write > 0)
      .sort((a, b) => b.cost - a.cost),
    total: days.reduce((a, d) => a + d.cost, 0),
    today: days.find((d) => d.day === today)?.cost ?? 0,
  };
}

// -- the card ----------------------------------------------------------------

/** Dollars to the cent, or to a tenth of a cent below a dollar: a day of
 *  Haiku is often less than one. */
export function dollars(n: number | null): string {
  if (n === null) return `<span class="muted">—</span>`;
  if (n === 0) return "$0";
  return n < 1 ? `$${n.toFixed(3)}` : `$${n.toFixed(2)}`;
}

/** Tokens as 1.2M / 34K / 512. */
export function tokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}K`;
  return String(n);
}

const short = (model: string) => model.replace(/^claude-/, "");

function reportedSection(r: Reported): string {
  if (r.days.length === 0) {
    return `<p class="sub">Nobody has reported a Claude request yet. Only copies released after 0.4.18
      send it, and only those with an Anthropic key set.</p>`;
  }
  const models = [...new Set(r.days.map((d) => d.model))];
  const today = r.days[0]?.day;
  const sum = (rows: ReportedDay[]) =>
    rows.reduce(
      (a, d) => ({
        requests: a.requests + d.requests,
        failed: a.failed + d.failed,
        input: a.input + d.input,
        output: a.output + d.output,
        cache_read: a.cache_read + d.cache_read,
        cache_write: a.cache_write + d.cache_write,
      }),
      { requests: 0, failed: 0, input: 0, output: 0, cache_read: 0, cache_write: 0 },
    );
  const week = new Date(Date.now() - 6 * DAY * 1000).toISOString().slice(0, 10);
  const modelRows = models
    .map((m) => {
      const rows = r.days.filter((d) => d.model === m);
      const all = sum(rows);
      const cost = listCost(m, all);
      const weekCost = listCost(m, sum(rows.filter((d) => d.day >= week)));
      return `<tr><td><b>${escapeHtml(short(m))}</b></td>
        <td class="num">${r.installs[m] ?? 0}</td>
        <td class="num">${all.requests}</td>
        <td class="num">${all.failed ? `<b>${all.failed}</b>` : "0"}</td>
        <td class="num">${tokens(all.input + all.cache_read + all.cache_write)}</td>
        <td class="num">${tokens(all.output)}</td>
        <td class="num">${dollars(weekCost)}</td>
        <td class="num"><b>${dollars(cost)}</b></td>
        <td class="num">${cost !== null && all.requests ? dollars(cost / all.requests) : `<span class="muted">—</span>`}</td></tr>`;
    })
    .join("");
  const dayList = [...new Set(r.days.map((d) => d.day))].slice(0, 14);
  const dayRows = dayList
    .map((day) => {
      const rows = r.days.filter((d) => d.day === day);
      const all = sum(rows);
      const costs = rows.map((d) => listCost(d.model, d));
      const cost = costs.every((c) => c === null) ? null : costs.reduce<number>((a, c) => a + (c ?? 0), 0);
      return `<tr><td>${escapeHtml(day)}${day === today ? ` <span class="muted">latest</span>` : ""}</td>
        <td class="num">${Math.max(...rows.map((d) => d.installs))}</td>
        <td class="num">${all.requests}</td>
        <td class="num">${all.failed ? `<b>${all.failed}</b>` : "0"}</td>
        <td class="num">${tokens(all.input + all.cache_read + all.cache_write)}</td>
        <td class="num">${tokens(all.output)}</td>
        <td class="num">${dollars(cost)}</td></tr>`;
    })
    .join("");
  return `<div class="scroll"><table>
      <tr><th>Model</th><th class="num">Installs</th><th class="num">Requests</th><th class="num">Failed</th>
          <th class="num">Tokens in</th><th class="num">Tokens out</th><th class="num">7 days</th>
          <th class="num">30 days</th><th class="num">Per request</th></tr>
      ${modelRows}
    </table></div>
    <div class="scroll" style="margin-top:14px"><table>
      <tr><th>Day</th><th class="num">Installs</th><th class="num">Requests</th><th class="num">Failed</th>
          <th class="num">Tokens in</th><th class="num">Tokens out</th><th class="num">Est. cost</th></tr>
      ${dayRows}
    </table></div>`;
}

function accountSection(a: Account | string): string {
  if (typeof a === "string") {
    return `<p class="sub">${escapeHtml(a)} Create an Admin API key in the Claude Console
      (Settings → Admin keys) and set it with
      <code>wrangler secret put ANTHROPIC_ADMIN_KEY</code> to see your organisation's real bill here.</p>`;
  }
  if (a.models.length === 0) return `<p class="sub">Nothing billed in the last 30 days.</p>`;
  const peak = Math.max(0.0001, ...a.days.map((d) => d.cost));
  const bars = a.days
    .map((d) => `<div style="height:${Math.round((d.cost / peak) * 100)}%" title="${escapeHtml(d.day)}: $${d.cost.toFixed(3)}"></div>`)
    .join("");
  const rows = a.models
    .map(
      (m) => `<tr><td>${escapeHtml(short(m.model))}</td>
        <td class="num">${tokens(m.input + m.cache_read + m.cache_write)}</td>
        <td class="num">${tokens(m.output)}</td>
        <td class="num"><b>${dollars(m.cost)}</b></td></tr>`,
    )
    .join("");
  return `<p class="sub">Today (UTC) <b>${dollars(a.today)}</b> · last 30 days <b>${dollars(a.total)}</b></p>
    <div class="bars">${bars}</div>
    <div class="bars-axis"><span>30 days ago</span><span>today</span></div>
    <div class="scroll" style="margin-top:14px"><table>
      <tr><th>Model</th><th class="num">Tokens in</th><th class="num">Tokens out</th><th class="num">Billed</th></tr>
      ${rows}
    </table></div>`;
}

export function claudeCard(reported: Reported, account: Account | string): string {
  return `<section class="card" id="claude">
       <p class="label">Claude — your users' keys, last 30 days</p>
       ${reportedSection(reported)}
       <p class="muted foot">Reported by the app in its daily ping: requests and tokens on the user's own
         Anthropic key, never the text. A day arrives with the next day's first ping, so the latest day
         fills in late. <b>Cost</b> is an estimate at list price (Haiku 5.5 $0.10 / $0.50 per million
         tokens in / out); users are billed by Anthropic, not by you. Your own machines are left out.</p>

       <p class="label" style="margin-top:22px">Claude — your Anthropic account, last 30 days</p>
       ${accountSection(account)}
       <p class="muted foot">Read live from Anthropic's Usage and Cost API: what your own organisation was
         billed, development and testing included. It cannot see your users' keys.</p>
     </section>`;
}
