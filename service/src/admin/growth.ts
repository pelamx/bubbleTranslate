// Where copies come from, who goes on to pay, and who stops.
//
// Three questions the rest of the panel answers only in part: how many copies
// are store installs rather than website downloads; which countries' users
// buy (the sources card already splits Pro by download source); and what
// share of subscribers each month ends -- the counts are in the revenue card,
// the rate is not.

import type { Env } from "../env";
import { escapeHtml } from "../pages";
import { NOT_MINE_INSTALL, NOT_MINE_LICENCE, PLATFORMS, ignoreList, mineInstalls, osLabel, ratio } from "./queries";
import { DAY } from "./shared";

// -- channels ---------------------------------------------------------------

export interface ChannelRow {
  os: string;
  channel: string;
  n: number;
}

/** Installs seen in the last 30 days per platform and channel. `unknown` is
 *  a copy too old to say. */
export async function channels(env: Env): Promise<ChannelRow[]> {
  try {
    const { results } = await env.DB.prepare(
      `SELECT COALESCE(os, 'unknown') AS os, COALESCE(channel, 'unknown') AS channel, COUNT(*) AS n
         FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}
        GROUP BY 1, 2`,
    )
      .bind(Math.floor(Date.now() / 1000) - 30 * DAY, null, null, null, null, null, null, null, await mineInstalls(env))
      .all<ChannelRow>();
    return results ?? [];
  } catch (err) {
    // `channel` is newer than the table; until it is added, nothing to show.
    console.error("could not read install channels", err);
    return [];
  }
}

export function channelsCard(rows: ChannelRow[]): string {
  const count = (os: string, channel: string) =>
    rows.filter((r) => r.os === os && r.channel === channel).reduce((a, r) => a + r.n, 0);
  const body = PLATFORMS.map((os) => {
    const all = rows.filter((r) => r.os === os).reduce((a, r) => a + r.n, 0);
    return `<tr><td>${escapeHtml(osLabel(os))}</td>
      <td class="num">${count(os, "download")}</td>
      <td class="num">${count(os, "store") ? `<b>${count(os, "store")}</b>` : "0"}</td>
      <td class="num"><span class="muted">${count(os, "unknown")}</span></td>
      <td>${ratio(count(os, "store"), all)}</td></tr>`;
  }).join("");
  return `<section class="card" id="channels">
       <p class="label">Website or store, installs active in the last 30 days</p>
       <div class="scroll"><table>
         <tr><th>Platform</th><th class="num">Website</th><th class="num">Store</th>
             <th class="num">Not reported</th><th>Store share</th></tr>
         ${body}
       </table></div>
       <p class="muted foot"><b>Store</b> is the Microsoft Store on Windows and the Mac App Store on macOS. Copies
         released up to 0.4.18 do not say, and are <b>not reported</b> until they update. A store install is never
         matched to a website download click.</p>
     </section>`;
}

// -- conversion by country ----------------------------------------------------

export interface CountryConversion {
  country: string;
  installs: number;
  pro: number;
}

/** Installs active in the last 30 days by country, and how many run Pro. */
export async function conversionByCountry(env: Env): Promise<CountryConversion[]> {
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(country, '?') AS country, COUNT(*) AS installs,
            SUM(CASE WHEN plan = 'pro' THEN 1 ELSE 0 END) AS pro
       FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}
      GROUP BY 1 ORDER BY installs DESC, pro DESC LIMIT 20`,
  )
    .bind(Math.floor(Date.now() / 1000) - 30 * DAY, null, null, null, null, null, null, null, await mineInstalls(env))
    .all<CountryConversion>();
  return results ?? [];
}

// -- churn --------------------------------------------------------------------

export interface ChurnMonth {
  month: string;
  /** Paid licences live on the first of the month. */
  start: number;
  /** Of those, ended by the first of the next: cancelled and run out, or refunded. */
  ended: number;
}

/** The last six calendar months. A licence counts as live on a day when it
 *  had begun and not yet run out; one that was live at the start and is not
 *  at the end of the month has ended in it, whether by running out after a
 *  cancellation or by a refund. Renewals extend `expires_at`, so a renewed
 *  licence never reads as ended. */
export async function churn(env: Env): Promise<ChurnMonth[]> {
  const d = new Date();
  const starts = Array.from({ length: 7 }, (_, i) => Date.UTC(d.getUTCFullYear(), d.getUTCMonth() - 5 + i, 1) / 1000);
  const mine = ignoreList(env.ADMIN_IGNORE_EMAILS);
  const now = Math.floor(Date.now() / 1000);
  const rows = await env.DB.batch(
    starts.slice(0, 6).map((start, i) =>
      env.DB.prepare(
        `SELECT COUNT(*) AS start,
                SUM(CASE WHEN expires_at <= ?2 THEN 1 ELSE 0 END) AS ended
           FROM licences
          WHERE provider = 'paddle' AND created_at < ?1 AND expires_at > ?1 AND ${NOT_MINE_LICENCE}`,
      ).bind(start, Math.min(starts[i + 1], now), null, null, null, null, null, null, mine),
    ),
  );
  return starts.slice(0, 6).map((start, i) => {
    const r = (rows[i]?.results?.[0] ?? {}) as { start?: number; ended?: number | null };
    return { month: new Date(start * 1000).toISOString().slice(0, 7), start: r.start ?? 0, ended: r.ended ?? 0 };
  }).reverse();
}

export function conversionCard(countries: CountryConversion[], months: ChurnMonth[]): string {
  const countryRows = countries
    .filter((c) => c.installs >= 3)
    .map(
      (c) => `<tr><td>${escapeHtml(c.country === "?" ? "not recorded" : c.country)}</td>
        <td class="num">${c.installs}</td><td class="num">${c.pro}</td>
        <td>${ratio(c.pro, c.installs)}</td></tr>`,
    )
    .join("");
  const churnRows = months
    .map(
      (m, i) => `<tr><td>${escapeHtml(m.month)}${i === 0 ? ` <span class="muted">so far</span>` : ""}</td>
        <td class="num">${m.start}</td><td class="num">${m.ended}</td>
        <td>${ratio(m.ended, m.start)}</td></tr>`,
    )
    .join("");
  return `<section class="card" id="conversion">
       <p class="label">Who pays, by country — installs active in the last 30 days</p>
       ${
         countryRows
           ? `<div class="scroll"><table>
         <tr><th>Country</th><th class="num">Installs</th><th class="num">On Pro</th><th>Converted</th></tr>
         ${countryRows}
       </table></div>`
           : `<p class="sub">No country has three active installs yet.</p>`
       }
       <p class="label" style="margin-top:22px">Churn — paid licences ending each month</p>
       <div class="scroll"><table>
         <tr><th>Month</th><th class="num">Live on the 1st</th><th class="num">Ended in it</th><th>Churn</th></tr>
         ${churnRows}
       </table></div>
       <p class="muted foot">Countries with fewer than three active installs are left out. Conversion by download
         source is in <b>Where users come from</b>. <b>Ended</b> is a licence that was live on the first and is not
         at the month's end — a cancellation that ran out, or a refund; a renewal is not an end. Your own machines
         and test purchases are left out.</p>
     </section>`;
}
