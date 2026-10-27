// Translations a day, and how much of the free allowance they use.
//
// Two sources, because they reach back differently. Every copy since 0.3.6
// reports which backend answered (`provider_health`), so a backend's
// answers added up are the day's translations across every version -- but
// with no install id, so they cannot be split by plan or have your own
// machines taken out. Copies released after 0.4.21 also report, per install
// and day, how many translations they made on which plan, how many were
// charged to the ten free ones and how many were refused (`translate_usage`).

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { NOT_MINE_INSTALL, mineInstalls } from "./queries";
import { DAY } from "./shared";

/** The free plan's translations a day; `FREE_DAILY_TRANSLATIONS` in the app. */
const FREE_LIMIT = 10;

export interface UsageDay {
  day: string;
  /** Answers from every backend, every version. */
  all: number;
  free_installs: number;
  free_done: number;
  free_charged: number;
  free_capped: number;
  /** Free installs that spent all ten, or were refused, that day. */
  free_full: number;
  pro_installs: number;
  pro_done: number;
}

/** Free install-days by how many of the ten they spent. */
export interface UsageSpread {
  low: number;
  mid: number;
  high: number;
  full: number;
}

export interface Usage {
  days: UsageDay[];
  spread: UsageSpread;
}

const empty = (): Usage => ({ days: [], spread: { low: 0, mid: 0, high: 0, full: 0 } });

/** The last 30 days. */
export async function translateUse(env: Env): Promise<Usage> {
  const t = Math.floor(Date.now() / 1000);
  const since = new Date((t - 29 * DAY) * 1000).toISOString().slice(0, 10);
  const mine = await mineInstalls(env);
  try {
    const [all, split, spread] = await Promise.all([
      env.DB.prepare(`SELECT day, SUM(ok) AS n FROM provider_health WHERE day >= ?1 GROUP BY day`)
        .bind(since)
        .all<{ day: string; n: number }>(),
      env.DB.prepare(
        `SELECT day,
                SUM(CASE WHEN plan = 'free' THEN 1 ELSE 0 END) AS free_installs,
                SUM(CASE WHEN plan = 'free' THEN done ELSE 0 END) AS free_done,
                SUM(CASE WHEN plan = 'free' THEN charged ELSE 0 END) AS free_charged,
                SUM(CASE WHEN plan = 'free' THEN capped ELSE 0 END) AS free_capped,
                SUM(CASE WHEN plan = 'free' AND (charged >= ${FREE_LIMIT} OR capped > 0) THEN 1 ELSE 0 END) AS free_full,
                SUM(CASE WHEN plan = 'pro' THEN 1 ELSE 0 END) AS pro_installs,
                SUM(CASE WHEN plan = 'pro' THEN done ELSE 0 END) AS pro_done
           FROM translate_usage
          WHERE day >= ?1 AND ${NOT_MINE_INSTALL}
          GROUP BY day`,
      )
        .bind(since, null, null, null, null, null, null, null, mine)
        .all<Omit<UsageDay, "day" | "all"> & { day: string }>(),
      env.DB.prepare(
        `SELECT SUM(CASE WHEN charged BETWEEN 1 AND 3 AND capped = 0 THEN 1 ELSE 0 END) AS low,
                SUM(CASE WHEN charged BETWEEN 4 AND 6 AND capped = 0 THEN 1 ELSE 0 END) AS mid,
                SUM(CASE WHEN charged BETWEEN 7 AND ${FREE_LIMIT - 1} AND capped = 0 THEN 1 ELSE 0 END) AS high,
                SUM(CASE WHEN charged >= ${FREE_LIMIT} OR capped > 0 THEN 1 ELSE 0 END) AS full
           FROM translate_usage
          WHERE plan = 'free' AND day >= ?1 AND ${NOT_MINE_INSTALL}`,
      )
        .bind(since, null, null, null, null, null, null, null, mine)
        .first<UsageSpread>(),
    ]);
    const byDay = new Map<string, UsageDay>();
    const zero = (day: string): UsageDay => ({
      day, all: 0, free_installs: 0, free_done: 0, free_charged: 0, free_capped: 0, free_full: 0, pro_installs: 0, pro_done: 0,
    });
    for (const r of all.results ?? []) byDay.set(r.day, { ...zero(r.day), all: r.n ?? 0 });
    for (const r of split.results ?? []) byDay.set(r.day, { ...(byDay.get(r.day) ?? zero(r.day)), ...r });
    return {
      days: [...byDay.values()].sort((a, b) => (a.day < b.day ? 1 : -1)),
      spread: {
        low: spread?.low ?? 0,
        mid: spread?.mid ?? 0,
        high: spread?.high ?? 0,
        full: spread?.full ?? 0,
      },
    };
  } catch (err) {
    // The table is new; a database without it must not take the panel down.
    console.error("could not read translate_usage", err);
    return empty();
  }
}

export function usageCard(u: Usage): string {
  if (u.days.length === 0) {
    return `<section class="card" id="usage">
       <p class="label">Translations a day, last 30 days</p>
       <p class="sub">Nothing reported yet.</p>
     </section>`;
  }
  const muted = (n: number) => (n ? String(n) : `<span class="muted">0</span>`);
  const avg = (part: number, whole: number) => (whole ? (part / whole).toFixed(1) : `<span class="muted">—</span>`);
  const rows = u.days
    .slice(0, 14)
    .map(
      (d) => `<tr><td>${escapeHtml(d.day)}</td>
        <td class="num"><b>${d.all}</b></td>
        <td class="num">${muted(d.free_done)} <span class="muted">by ${d.free_installs}</span></td>
        <td class="num">${avg(d.free_charged, d.free_installs)}</td>
        <td class="num">${d.free_full ? `<b>${d.free_full}</b>` : muted(0)}</td>
        <td class="num">${muted(d.free_capped)}</td>
        <td class="num">${muted(d.pro_done)} <span class="muted">by ${d.pro_installs}</span></td></tr>`,
    )
    .join("");
  const s = u.spread;
  const days = s.low + s.mid + s.high + s.full;
  const share = (n: number) =>
    days ? `<b>${Math.round((n / days) * 100)}%</b> <span class="muted">${n}</span>` : `<span class="muted">—</span>`;
  return `<section class="card" id="usage">
       <p class="label">Translations a day, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Free days that used 1–3 of ${FREE_LIMIT}</th><th>4–6</th><th>7–${FREE_LIMIT - 1}</th>
             <th>All ${FREE_LIMIT}</th></tr>
         <tr><td>${share(s.low)}</td><td>${share(s.mid)}</td><td>${share(s.high)}</td><td>${share(s.full)}</td></tr>
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Day</th><th class="num">All versions</th><th class="num">Free</th>
             <th class="num">Free used, average</th><th class="num">Reached ${FREE_LIMIT}</th>
             <th class="num">Refused</th><th class="num">Pro</th></tr>
         ${rows}
       </table></div>
       <p class="muted foot"><b>All versions</b> is every translation a backend answered, from every copy since
         0.3.6, your own machines included. The other columns come only from copies released after 0.4.21,
         without your machines: <b>Free used</b> is how many of the ${FREE_LIMIT} a free install spent that day
         (a repeat is not charged), <b>Reached ${FREE_LIMIT}</b> is how many installs spent them all, and
         <b>Refused</b> is translations asked for after that. "by" says how many installs. Reported in the ping,
         by copies with usage pings on.</p>
     </section>`;
}
