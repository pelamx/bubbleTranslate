// The website's own traffic: every page opened on bubbletranslate.app, sent
// by the page as a beacon (`page_views`). A visit is a view that did not come
// from another page of the site.

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { now } from "../tokens";
import { DAY, LOCAL_OFFSET } from "./shared";

export interface Visits {
  days: { day: string; views: number; visits: number }[];
  pages: { path: string; views: number }[];
  sources: { src: string; visits: number }[];
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
        `SELECT src, COUNT(*) AS visits FROM page_views WHERE at >= ?1 AND src != 'internal'
          GROUP BY src ORDER BY visits DESC LIMIT 15`,
      ).bind(since),
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
  const srcRows = v.sources
    .map((s) => `<tr><td>${s.src === "direct" ? `direct <span class="muted">— typed in, or no referrer</span>` : `<b>${escapeHtml(s.src)}</b>`}</td>
      <td class="num">${s.visits}</td></tr>`)
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
         <tr><th>Came from</th><th class="num">Visits</th></tr>
         ${srcRows || `<tr><td colspan="2" class="muted">Only moves between pages so far.</td></tr>`}
       </table></div>
       <p class="muted foot">Counted by the pages themselves, so crawlers that run no script are left out, and
         so is anyone blocking scripts. A <b>visit</b> is a page opened from outside the site; moving between
         its pages adds pages, not visits. No cookie and no identifier, so one person coming back
         twice is two visits. Your own visits are counted too.</p>
     </section>`;
}
