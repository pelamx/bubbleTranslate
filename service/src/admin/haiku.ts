// What Claude Haiku costs on your own key: the word-in-sentence relay, and how
// much credit is left to pay for it.
//
// Every copy without an Anthropic key of its own translates a single word in
// its sentence through `word.ts`, on the service's key, rationed per install
// and for everyone per UTC day. `word_usage` says how many words and tokens
// each day took.
//
// Anthropic has no API for the credit balance -- only the Console shows it --
// so the operator types it in from there now and then (`anthropic_balance`),
// and this subtracts what the Cost API says was billed since. Between two
// readings the figure drifts only by what the Cost API has not caught up on,
// which is a few hours at most.

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { WORD_DAILY, WORD_PER_INSTALL } from "../word";
import { CLAUDE_PRICES, anthropicSpentSince, dollars, tokens } from "./claude";
import { cached } from "./shared";

export interface WordDay {
  day: string;
  installs: number;
  requests: number;
  input: number;
  output: number;
  /** Installs that asked past their daily share and were refused. */
  over: number;
}

/** The last 30 UTC days of the relay, newest first. Everyone is counted, the
 *  operator's own machines included: they spend the same key. */
export async function wordUse(env: Env): Promise<WordDay[]> {
  try {
    const rows = await env.DB.prepare(
      `SELECT day, COUNT(*) AS installs, SUM(requests) AS requests,
              SUM(input) AS input, SUM(output) AS output,
              SUM(CASE WHEN requests > ?1 THEN 1 ELSE 0 END) AS over
         FROM word_usage GROUP BY day ORDER BY day DESC LIMIT 30`,
    )
      .bind(WORD_PER_INSTALL)
      .all<WordDay>();
    return rows.results ?? [];
  } catch (err) {
    // A database the schema has not reached yet must not take the panel down.
    console.error("could not read word_usage", err);
    return [];
  }
}

/** Words refused past an install's share are counted in `requests` but never
 *  reached Claude; the day's words are the ones that did, near enough. */
const haiku = CLAUDE_PRICES["claude-haiku-5-5"];
const wordCost = (d: { input: number; output: number }) =>
  (d.input * haiku.input + d.output * haiku.output) / 1e6;

export interface Credit {
  /** What the Console said, and when it was typed in. */
  entered: number;
  at: number;
  /** Billed since, from the Cost API; a string when it could not be read. */
  spent: number | string;
}

/** The last balance typed in, with what was billed since. Null before the
 *  first. */
export async function credit(env: Env): Promise<Credit | null> {
  let row: { dollars: number; at: number } | null = null;
  try {
    row = await env.DB.prepare("SELECT dollars, at FROM anthropic_balance WHERE id = 1").first();
  } catch (err) {
    console.error("could not read anthropic_balance", err);
  }
  if (!row) return null;
  const spent = await cached(`anthropic-spent-${row.at}`, 300, () => anthropicSpentSince(env, row.at));
  return { entered: row.dollars, at: row.at, spent };
}

/** Saves the balance as the Console shows it now. */
export async function setCredit(env: Env, dollars: number, at: number): Promise<void> {
  await env.DB.prepare(
    `INSERT INTO anthropic_balance (id, dollars, at) VALUES (1, ?1, ?2)
     ON CONFLICT (id) DO UPDATE SET dollars = ?1, at = ?2`,
  )
    .bind(dollars, at)
    .run();
}

const n = (x: number) => x.toLocaleString("en-US");

function creditSection(c: Credit | null, perDay: number | null): string {
  const form = `<form method="post" action="/admin/anthropic-balance" class="row" style="margin-top:10px">
      <input name="dollars" inputmode="decimal" placeholder="Balance in the Console, e.g. 18.40" required
             style="max-width:240px">
      <button type="submit">Save balance</button>
    </form>`;
  if (!c) {
    return `<p class="sub">No balance entered yet. Anthropic does not let a program read it: copy the
      remaining credit from <a href="https://console.anthropic.com/settings/billing" target="_blank"
      rel="noopener">Console → Billing</a> into the box, and from then on this card counts it down.</p>${form}`;
  }
  const when = new Date(c.at * 1000).toISOString().slice(0, 16).replace("T", " ");
  if (typeof c.spent === "string") {
    return `<p class="sub">Entered <b>${dollars(c.entered)}</b> on ${escapeHtml(when)} UTC ·
      <span class="muted">${escapeHtml(c.spent)}</span></p>${form}`;
  }
  const left = c.entered - c.spent;
  const days = perDay && perDay > 0 ? Math.floor(Math.max(0, left) / perDay) : null;
  const low = left <= 0 || (days !== null && days < 14);
  return `<p class="sub" style="font-size:1.15em">About <b${low ? ' class="warn"' : ""}>${dollars(Math.max(0, left))}</b> left${
    days !== null ? ` · enough for about <b${low ? ' class="warn"' : ""}>${n(days)} days</b> at last week's pace` : ""
  }</p>
    <p class="sub">${dollars(c.entered)} in the Console on ${escapeHtml(when)} UTC, ${dollars(c.spent)} billed since.
      Type the Console's figure in again after topping up.</p>${form}`;
}

/** `bill` is the organisation's daily cost from the account card, for the
 *  pace; null when it could not be read. */
export function haikuCard(days: WordDay[], c: Credit | null, bill: { day: string; cost: number }[] | null): string {
  const today = new Date().toISOString().slice(0, 10);
  // The pace: the last seven whole days of what the organisation was billed,
  // everything included -- the relay, your own testing, anything else.
  const whole = (bill ?? []).filter((d) => d.day < today).slice(-7);
  const perDay = whole.length ? whole.reduce((a, d) => a + d.cost, 0) / whole.length : null;

  const todayRow = days.find((d) => d.day === today);
  const share = (todayRow?.requests ?? 0) / WORD_DAILY;
  const total = days.reduce(
    (a, d) => ({ requests: a.requests + d.requests, input: a.input + d.input, output: a.output + d.output }),
    { requests: 0, input: 0, output: 0 },
  );
  const peak = Math.max(1, ...days.map((d) => d.requests));
  const bars = [...days]
    .reverse()
    .map((d) => `<div style="height:${Math.round((d.requests / peak) * 100)}%" title="${escapeHtml(d.day)}: ${n(d.requests)} words"></div>`)
    .join("");
  const rows = days
    .slice(0, 14)
    .map(
      (d) => `<tr><td>${escapeHtml(d.day)}${d.day === today ? ` <span class="muted">today</span>` : ""}</td>
        <td class="num">${n(d.installs)}</td>
        <td class="num">${n(d.requests)}</td>
        <td class="num">${tokens(d.input)} / ${tokens(d.output)}</td>
        <td class="num">${dollars(wordCost(d))}</td>
        <td class="num">${d.over ? `<b>${d.over}</b>` : "0"}</td></tr>`,
    )
    .join("");
  return `<section class="card" id="haiku">
       <p class="label">Claude Haiku — your key: credit left</p>
       ${creditSection(c, perDay)}

       <p class="label" style="margin-top:22px">Words translated in their sentence, last 30 days</p>
       <p class="sub">Today <b>${n(todayRow?.requests ?? 0)}</b> of ${n(WORD_DAILY)} words
         (<b${share >= 0.8 ? ' class="warn"' : ""}>${Math.round(share * 100)}%</b> of the daily cap) ·
         ${n(todayRow?.installs ?? 0)} installs · 30 days <b>${n(total.requests)}</b> words,
         about <b>${dollars(wordCost(total))}</b></p>
       ${
         days.length
           ? `<div class="bars">${bars}</div>
       <div class="bars-axis"><span>${escapeHtml(days[days.length - 1].day)}</span><span>today</span></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Day (UTC)</th><th class="num">Installs</th><th class="num">Words</th>
             <th class="num">Tokens in / out</th><th class="num">Cost</th><th class="num">Hit own limit</th></tr>
         ${rows}
       </table></div>`
           : `<p class="sub">No word has been translated through the service yet.</p>`
       }
       <p class="muted foot">Copies without an Anthropic key of their own send a selected word and its sentence
         here, and the service asks Claude Haiku on your key. Each install may have ${WORD_PER_INSTALL} words a
         day and everyone together ${n(WORD_DAILY)}; past either, the word is translated without Claude.
         <b>Cost</b> is at list price ($0.10 / $0.50 per million tokens in / out). <b>Credit left</b> is the
         Console balance you entered minus what Anthropic's Cost API says was billed since, for everything on
         your organisation, not only this.</p>
     </section>`;
}
