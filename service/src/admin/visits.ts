// The website's own traffic: every page opened on bubbletranslate.app, sent
// by the page as a beacon (`page_views`). A visit is a view that did not come
// from another page of the site.

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { now } from "../tokens";
import { DAY, LOCAL_OFFSET, startOfToday } from "./shared";

export interface Visits {
  days: { day: string; views: number; visits: number }[];
  pages: { path: string; views: number }[];
  sources: { src: string; visits: number; week: number; today: number }[];
}

/** The last 30 days, by the operator's local day. */
export async function siteVisits(env: Env): Promise<Visits> {
  const since = now() - 30 * DAY;
  try {
    const [days, pages, sources] = await env.DB.batch([
      env.DB.prepare(
        `SELECT date(at + ?1, 'unixepoch') AS day, COUNT(*) AS views,
                SUM(src != 'internal') AS visits
           FROM page_views WHERE at >= ?2 GROUP BY day ORDER BY day DESC`,
      ).bind(LOCAL_OFFSET, since),
      env.DB.prepare(
        `SELECT path, COUNT(*) AS views FROM page_views WHERE at >= ?1
          GROUP BY path ORDER BY views DESC LIMIT 15`,
      ).bind(since),
      env.DB.prepare(
        `SELECT src, COUNT(*) AS visits, SUM(at >= ?2) AS week, SUM(at >= ?3) AS today
           FROM page_views WHERE at >= ?1 AND src != 'internal'
          GROUP BY src ORDER BY visits DESC LIMIT 25`,
      ).bind(since, now() - 7 * DAY, startOfToday()),
    ]);
    return {
      days: (days.results ?? []) as Visits["days"],
      pages: (pages.results ?? []) as Visits["pages"],
      sources: (sources.results ?? []) as Visits["sources"],
    };
  } catch (err) {
    // The table is new; a database without it must not take the panel down.
    console.error("could not read page_views", err);
    return { days: [], pages: [], sources: [] };
  }
}

/** A traffic source as the panel shows it. */
export function sourceName(src: string): string {
  if (src === "google-ads") return `<b>Google Ads</b>`;
  if (src === "google") return `<b>Google search</b>`;
  if (src === "direct") return `direct <span class="muted">— typed in, or no referrer</span>`;
  return `<b>${escapeHtml(src)}</b>`;
}

export function visitsCard(v: Visits): string {
  if (v.days.length === 0) {
    return `<section class="card" id="visits">
       <p class="label">Website visits, last 30 days</p>
       <p class="sub">Nothing counted yet. Pages report themselves from 4 November 2026.</p>
     </section>`;
  }
  const views = v.days.reduce((a, d) => a + d.views, 0);
  const visits = v.days.reduce((a, d) => a + d.visits, 0);
  const dayRows = v.days
    .slice(0, 14)
    .map((d) => `<tr><td>${escapeHtml(d.day)}</td><td class="num"><b>${d.visits}</b></td><td class="num">${d.views}</td></tr>`)
    .join("");
  const pageRows = v.pages
    .map((p) => `<tr><td><code>${escapeHtml(p.path)}</code></td><td class="num">${p.views}</td></tr>`)
    .join("");
  const most = Math.max(1, ...v.sources.map((s) => s.visits));
  const srcRows = v.sources
    .map((s) => `<tr><td>${sourceName(s.src)}</td>
      <td class="num">${s.today || `<span class="muted">0</span>`}</td><td class="num">${s.week}</td>
      <td class="num"><b>${s.visits}</b></td><td class="num">${Math.round((s.visits / visits) * 100)}%</td>
      <td style="width:18%"><span class="track"><i style="width:${Math.round((s.visits / most) * 100)}%;background:var(--brand)"></i></span></td></tr>`)
    .join("");
  return `<section class="card" id="visits">
       <p class="label">Website visits, last 30 days</p>
       <p class="sub"><b>${visits}</b> visits · ${views} pages opened</p>
       <div class="scroll"><table>
         <tr><th>Day</th><th class="num">Visits</th><th class="num">Pages opened</th></tr>
         ${dayRows}
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Page</th><th class="num">Opened</th></tr>
         ${pageRows}
       </table></div>
       <div class="scroll" style="margin-top:14px"><table>
         <tr><th>Came from</th><th class="num">Today</th><th class="num">7 days</th><th class="num">30 days</th><th class="num">Share</th><th></th></tr>
         ${srcRows || `<tr><td colspan="6" class="muted">Only moves between pages so far.</td></tr>`}
       </table></div>
       <p class="muted foot">Counted by the pages themselves, so crawlers that run no script are left out, and
         so is anyone blocking scripts. A <b>visit</b> is a page opened from outside the site; moving between
         its pages adds pages, not visits. No cookie and no identifier, so one person coming back
         twice is two visits. Your own visits are counted too. <b>Google Ads</b> is a visit that arrived
         with an ad click id; before 11 November 2026 they and every Google search were counted under google.com.</p>
     </section>`;
}
