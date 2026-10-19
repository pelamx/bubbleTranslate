// Screen readings: a box (Ctrl+Shift+E) or the whole screen (Ctrl+Shift+F).
//
// The free plan has three a day of its own, apart from the ten translations.
// Whether that is the right number depends on how many people use the
// feature at all, and how many run into the three -- which is what the daily
// ping now reports per install and day (`screen_usage`).

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { NOT_MINE_INSTALL, mineInstalls, ratio } from "./queries";
import { DAY } from "./shared";

export interface ScreenDay {
  day: string;
  kind: string;
  installs: number;
  done: number;
  capped: number;
  /** Installs refused at least once that day. */
  capped_installs: number;
}

export interface ScreenTotals {
  /** Installs active in the window, the base for the shares. */
  active: number;
  box: number;
  page: number;
  /** Installs refused at least once in the window. */
  capped: number;
  /** Of those, on Pro now. */
  capped_pro: number;
}

export interface Screen {
  days: ScreenDay[];
  totals: ScreenTotals;
}

/** The last 30 days, the operator's own machines left out. */
export async function screenUse(env: Env): Promise<Screen> {
  const t = Math.floor(Date.now() / 1000);
  const since = new Date((t - 29 * DAY) * 1000).toISOString().slice(0, 10);
  const mine = await mineInstalls(env);
  try {
    const [days, totals, active] = await Promise.all([
      env.DB.prepare(
        `SELECT day, kind, COUNT(*) AS installs, SUM(done) AS done, SUM(capped) AS capped,
                SUM(CASE WHEN capped > 0 THEN 1 ELSE 0 END) AS capped_installs
           FROM screen_usage
          WHERE day >= ?1 AND ${NOT_MINE_INSTALL}
          GROUP BY day, kind ORDER BY day DESC, kind`,
      )
        .bind(since, null, null, null, null, null, null, null, mine)
        .all<ScreenDay>(),
      env.DB.prepare(
        `SELECT COUNT(DISTINCT CASE WHEN s.kind = 'box' AND s.done > 0 THEN s.install END) AS box,
                COUNT(DISTINCT CASE WHEN s.kind = 'page' AND s.done > 0 THEN s.install END) AS page,
                COUNT(DISTINCT CASE WHEN s.capped > 0 THEN s.install END) AS capped,
                COUNT(DISTINCT CASE WHEN s.capped > 0 AND i.plan = 'pro' THEN s.install END) AS capped_pro
           FROM screen_usage s LEFT JOIN installs i ON i.install = s.install
          WHERE s.day >= ?1
            AND s.install NOT IN (SELECT value FROM json_each(?9))
            AND s.install NOT IN (SELECT install FROM ignored_installs)`,
      )
        .bind(since, null, null, null, null, null, null, null, mine)
        .first<Omit<ScreenTotals, "active">>(),
      env.DB.prepare(`SELECT COUNT(*) AS n FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}`)
        .bind(t - 30 * DAY, null, null, null, null, null, null, null, mine)
        .first<{ n: number }>(),
    ]);
    return {
      days: days.results ?? [],
      totals: {
        active: active?.n ?? 0,
        box: totals?.box ?? 0,
        page: totals?.page ?? 0,
        capped: totals?.capped ?? 0,
        capped_pro: totals?.capped_pro ?? 0,
      },
    };
  } catch (err) {
    // The table is new; a database without it must not take the panel down.
    console.error("could not read screen_usage", err);
    return { days: [], totals: { active: 0, box: 0, page: 0, capped: 0, capped_pro: 0 } };
  }
}

export function screenCard(s: Screen): string {
  if (s.days.length === 0) {
    return `<section class="card" id="screen">
       <p class="label">Screen readings, last 30 days</p>
       <p class="sub">Nothing reported yet. Only copies released after 0.4.18 send it.</p>
     </section>`;
  }
  const dayList = [...new Set(s.days.map((d) => d.day))].slice(0, 14);
  const cell = (day: string, kind: string) => {
    const d = s.days.find((x) => x.day === day && x.kind === kind);
    if (!d) return `<td class="num"><span class="muted">0</span></td><td class="num"><span class="muted">—</span></td>`;
    return `<td class="num">${d.done} <span class="muted">by ${d.installs}</span></td>
      <td class="num">${d.capped ? `<b>${d.capped}</b> <span class="muted">by ${d.capped_installs}</span>` : "0"}</td>`;
  };
  const rows = dayList
    .map((day) => `<tr><td>${escapeHtml(day)}</td>${cell(day, "box")}${cell(day, "page")}</tr>`)
    .join("");
  const t = s.totals;
  return `<section class="card" id="screen">
       <p class="label">Screen readings, last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Read a box</th><th>Read the whole screen</th><th>Ran out of free readings</th>
             <th>Of those, on Pro now</th></tr>
         <tr><td>${ratio(t.box, t.active)}</td><td>${ratio(t.page, t.active)}</td>
             <td>${ratio(t.capped, t.active)}</td><td>${ratio(t.capped_pro, t.capped)}</td></tr>
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Day</th><th class="num">Boxes read</th><th class="num">Box refused</th>
             <th class="num">Screens read</th><th class="num">Screen refused</th></tr>
         ${rows}
       </table></div>
       <p class="muted foot">Shares are of the installs active in the last 30 days. <b>Refused</b> is a reading
         asked for after the day's three free ones were spent — "by" says how many installs. Reported in the
         daily ping, a day after it happened, by copies released after 0.4.18. Your own machines are left out.</p>
     </section>`;
}
