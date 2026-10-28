// What the DeepL fallback spends: the one translation this service pays for.
//
// Every copy without a DeepL key of its own shares one free key (see
// `deepl.ts`), rationed per install and for everyone per UTC day. The rows in
// `deepl_usage` say how close each day came to the ration; DeepL's own
// `/v2/usage` says how much of the month's allowance is gone.

import { DEEPL_DAILY_CHARS, DEEPL_PER_INSTALL, DEEPL_PER_INSTALL_CHARS } from "../deepl";
import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { NOT_MINE_INSTALL, mineInstalls } from "./queries";
import { cached } from "./shared";

export interface DeeplDay {
  day: string;
  chars: number;
  requests: number;
  installs: number;
  /** Installs that used up their daily share. A refusal is not recorded, so
   *  this counts those that reached it rather than those turned away. */
  over: number;
}

export interface DeeplTop {
  install: string;
  requests: number;
  chars: number;
}

export interface DeeplUse {
  days: DeeplDay[];
  /** Today's heaviest installs. */
  top: DeeplTop[];
}

/** The last seven UTC days, which is all `deepl_usage` keeps. Everyone is
 *  counted, the operator's own machines included -- they spend the same key --
 *  but the heaviest-installs list leaves them out. */
export async function deeplUse(env: Env): Promise<DeeplUse> {
  const today = new Date().toISOString().slice(0, 10);
  const [days, top] = await Promise.all([
    env.DB.prepare(
      `SELECT day, SUM(chars) AS chars, SUM(requests) AS requests, COUNT(*) AS installs,
              SUM(CASE WHEN requests >= ?1 OR chars >= ?2 THEN 1 ELSE 0 END) AS over
         FROM deepl_usage GROUP BY day ORDER BY day DESC LIMIT 7`,
    )
      .bind(DEEPL_PER_INSTALL, DEEPL_PER_INSTALL_CHARS)
      .all<DeeplDay>(),
    env.DB.prepare(
      `SELECT install, requests, chars FROM deepl_usage
        WHERE day = ?1 AND ${NOT_MINE_INSTALL}
        ORDER BY chars DESC LIMIT 5`,
    )
      .bind(today, null, null, null, null, null, null, null, await mineInstalls(env))
      .all<DeeplTop>(),
  ]);
  return { days: days.results ?? [], top: top.results ?? [] };
}

export interface DeeplAccount {
  used: number;
  limit: number;
}

/** The key's billing period so far, from DeepL. A string is why it could not
 *  be read. */
export async function deeplAccount(env: Env): Promise<DeeplAccount | string> {
  const key = env.DEEPL_API_KEY?.trim();
  if (!key) return "No DeepL key is configured, so the fallback is off.";
  return cached("deepl-usage", 600, async () => {
    const host = key.endsWith(":fx") ? "https://api-free.deepl.com" : "https://api.deepl.com";
    try {
      const res = await fetch(`${host}/v2/usage`, {
        headers: { authorization: `DeepL-Auth-Key ${key}` },
        signal: AbortSignal.timeout(8000),
      });
      if (!res.ok) return `DeepL answered ${res.status}.`;
      const body: any = await res.json();
      return { used: Number(body?.character_count) || 0, limit: Number(body?.character_limit) || 0 };
    } catch {
      return "DeepL could not be reached just now.";
    }
  });
}

const n = (x: number) => x.toLocaleString("en-US");

export function deeplCard(use: DeeplUse, account: DeeplAccount | string): string {
  const today = new Date().toISOString().slice(0, 10);
  const todayRow = use.days.find((d) => d.day === today);
  const share = (todayRow?.chars ?? 0) / DEEPL_DAILY_CHARS;
  const month =
    typeof account === "string"
      ? `<span class="muted">${escapeHtml(account)}</span>`
      : `this month <b>${n(account.used)}</b> of ${n(account.limit)} characters
         (<b${account.limit && account.used / account.limit > 0.8 ? ' class="warn"' : ""}>${
           account.limit ? Math.round((account.used / account.limit) * 100) : 0
         }%</b>)`;
  const rows = use.days
    .map(
      (d) => `<tr><td>${escapeHtml(d.day)}${d.day === today ? ` <span class="muted">today</span>` : ""}</td>
        <td class="num">${d.installs}</td>
        <td class="num">${d.requests}</td>
        <td class="num">${n(d.chars)}</td>
        <td class="num">${d.chars >= DEEPL_DAILY_CHARS * 0.95 ? `<b class="warn">${Math.round((d.chars / DEEPL_DAILY_CHARS) * 100)}%</b>` : `${Math.round((d.chars / DEEPL_DAILY_CHARS) * 100)}%`}</td>
        <td class="num">${d.over ? `<b>${d.over}</b>` : "0"}</td></tr>`,
    )
    .join("");
  const top = use.top.length
    ? `<p class="label" style="margin-top:18px">Heaviest installs today</p>
       <div class="scroll"><table>
         <tr><th>Install</th><th class="num">Requests</th><th class="num">Characters</th></tr>
         ${use.top
           .map(
             (t) => `<tr><td><code>${escapeHtml(t.install.slice(0, 8))}</code></td>
               <td class="num">${t.requests}${t.requests >= DEEPL_PER_INSTALL || t.chars >= DEEPL_PER_INSTALL_CHARS ? ` <span class="badge">capped</span>` : ""}</td>
               <td class="num">${n(t.chars)}</td></tr>`,
           )
           .join("")}
       </table></div>`
    : "";
  return `<section class="card" id="deepl">
       <p class="label">DeepL fallback — your key, last 7 days</p>
       <p class="sub">Today ${n(todayRow?.chars ?? 0)} of ${n(DEEPL_DAILY_CHARS)} characters
         (<b${share >= 0.95 ? ' class="warn"' : ""}>${Math.round(share * 100)}%</b>) · ${month}</p>
       ${
         rows
           ? `<div class="scroll"><table>
         <tr><th>Day (UTC)</th><th class="num">Installs</th><th class="num">Requests</th>
             <th class="num">Characters</th><th class="num">Of daily cap</th><th class="num">Hit own limit</th></tr>
         ${rows}
       </table></div>`
           : `<p class="sub">Nobody has needed the fallback in the last week.</p>`
       }
       ${top}
       <p class="muted foot">Asked only when Google and MyMemory have both failed, by copies without a DeepL
         key of their own. Each install may make ${DEEPL_PER_INSTALL} requests and spend
         ${n(DEEPL_PER_INSTALL_CHARS)} characters a day, and everyone together
         ${n(DEEPL_DAILY_CHARS)} characters; past either, the fallback says it has run out for today. A day near
         the cap is a day Google and MyMemory were failing for a lot of people.</p>
     </section>`;
}
