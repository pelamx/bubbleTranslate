// What the panel reads: counts, installs, versions, downloads and licences.

import { type Env } from "../env";
import { type Licence } from "../licences";
import { now, sha256Hex } from "../tokens";
import { DAY, PAGE_SIZE, startOfToday } from "./shared";

export interface Stats {
  live: number;
  monthly: number;
  yearly: number;
  paddle: number;
  winding_down: number;
  expiring: number;
  fresh: number;
  total: number;
}

/** A comma-separated secret as a JSON array, for `json_each(?)` in SQL. */
export function ignoreList(value: string | undefined): string {
  return JSON.stringify(
    (value ?? "").split(",").map((v) => v.trim().toLowerCase()).filter(Boolean),
  );
}

/** Keeps the operator's own machines and licences out of the counts. */
export const NOT_MINE_INSTALL = `install NOT IN (SELECT value FROM json_each(?9))
  AND install NOT IN (SELECT install FROM ignored_installs)`;
export const NOT_MINE_LICENCE = `(email IS NULL OR LOWER(email) NOT IN (SELECT value FROM json_each(?9)))`;

export const PLATFORMS = ["windows", "linux", "macos"] as const;

export interface HealthRow {
  os: string;
  total: number;
  day_base: number;
  day_back: number;
  week_base: number;
  week_back: number;
  lost: number;
}

/** Whether people keep using it, per OS. The installs table keeps only the
 *  first and the latest ping, so "came back" means the latest ping is at least
 *  a day (or a week) after the first — a floor, never an overcount. Only
 *  installs old enough to have had the chance are in each base. "Lost" is
 *  seen in the last 30 days but not in the last 7. */
export async function health(env: Env): Promise<HealthRow[]> {
  const t = now();
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(os, 'unknown') AS os,
            COUNT(*) AS total,
            SUM(CASE WHEN first_seen < ?1 - 2 * ${DAY} THEN 1 ELSE 0 END) AS day_base,
            SUM(CASE WHEN first_seen < ?1 - 2 * ${DAY} AND last_seen >= first_seen + ${DAY} THEN 1 ELSE 0 END) AS day_back,
            SUM(CASE WHEN first_seen < ?1 - 8 * ${DAY} THEN 1 ELSE 0 END) AS week_base,
            SUM(CASE WHEN first_seen < ?1 - 8 * ${DAY} AND last_seen >= first_seen + 7 * ${DAY} THEN 1 ELSE 0 END) AS week_back,
            SUM(CASE WHEN last_seen < ?1 - 7 * ${DAY} AND last_seen >= ?1 - 30 * ${DAY} THEN 1 ELSE 0 END) AS lost
       FROM installs WHERE ${NOT_MINE_INSTALL}
      GROUP BY COALESCE(os, 'unknown')`,
  )
    .bind(t, null, null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_INSTALLS))
    .all<HealthRow>();
  return results ?? [];
}

/** Which version each install in use this month is on, by OS. */
export async function versions(env: Env): Promise<{ app: string; os: string; n: number }[]> {
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(app, '?') AS app, COALESCE(os, 'unknown') AS os, COUNT(*) AS n
       FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}
      GROUP BY app, os`,
  )
    .bind(now() - 30 * DAY, null, null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_INSTALLS))
    .all<{ app: string; os: string; n: number }>();
  return results ?? [];
}

/** Newest version first: "0.2.10" after "0.2.9". */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pb[i] || 0) - (pa[i] || 0);
    if (d) return d;
  }
  return 0;
}

/** Every release asset's download count on GitHub, summed per OS. Cached for
 *  fifteen minutes: the API allows sixty unauthenticated calls an hour, and a
 *  Worker's outbound address is shared. Null when GitHub cannot be reached,
 *  so the panel says so rather than showing zeros. */
export async function downloads(env: Env): Promise<Record<string, number> | null> {
  const url = "https://api.github.com/repos/pelamx/bubbleTranslate/releases?per_page=100";
  const cache = caches.default;
  const key = new Request(url);
  let res = await cache.match(key);
  if (!res) {
    try {
      const fresh = await fetch(url, {
        headers: {
          "user-agent": "bubbleTranslate-admin",
          accept: "application/vnd.github+json",
          ...(env.GITHUB_TOKEN ? { authorization: `Bearer ${env.GITHUB_TOKEN}` } : {}),
        },
      });
      if (!fresh.ok) {
        console.error(`github releases returned ${fresh.status}`);
        return null;
      }
      res = new Response(await fresh.text(), {
        headers: { "content-type": "application/json", "cache-control": "max-age=900" },
      });
      await cache.put(key, res.clone());
    } catch {
      return null;
    }
  }
  const releases = (await res.json()) as { assets: { name: string; download_count: number }[] }[];
  const out: Record<string, number> = { windows: 0, linux: 0, macos: 0 };
  for (const r of releases) {
    for (const a of r.assets) {
      const name = a.name.toLowerCase();
      if (name.endsWith(".zip") || name.endsWith(".exe")) out.windows += a.download_count;
      else if (name.endsWith(".dmg")) out.macos += a.download_count;
      else if (name.includes("linux")) out.linux += a.download_count;
    }
  }
  return out;
}

/** "3 / 10 · 30%", or a dash when nobody is old enough to count yet. */
export function ratio(part: number, whole: number): string {
  if (!whole) return `<span class="muted">—</span>`;
  return `<b>${Math.round((part / whole) * 100)}%</b> <span class="muted">${part}/${whole}</span>`;
}

export interface UserRow {
  install: string;
  os: string | null;
  app: string | null;
  plan: string | null;
  first_seen: number;
  last_seen: number;
  mine: number;
}

/** Every install seen in the last 30 days, newest first, each marked with
 *  whether the operator said it is theirs. */
export async function users(env: Env): Promise<UserRow[]> {
  const { results } = await env.DB.prepare(
    `SELECT install, os, app, plan, first_seen, last_seen,
            CASE WHEN install IN (SELECT value FROM json_each(?9))
                   OR install IN (SELECT install FROM ignored_installs)
                 THEN 1 ELSE 0 END AS mine
       FROM installs
      WHERE last_seen > ?1
      ORDER BY first_seen DESC
      LIMIT 300`,
  )
    .bind(now() - 30 * DAY, null, null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_INSTALLS))
    .all<UserRow>();
  return results ?? [];
}

/** "5m ago", "3h ago", "2d ago". */
export interface Usage {
  daily: number;
  weekly: number;
  monthly: number;
  free_weekly: number;
  new_weekly: number;
  total: number;
}

/** Installs by their last daily ping. Counts people running the app, free and
 *  Pro alike; a download that was never opened is not in here. */
export async function usage(env: Env): Promise<Usage> {
  const t = now();
  const row = await env.DB.prepare(
    `SELECT
       SUM(CASE WHEN last_seen > ?1 THEN 1 ELSE 0 END) AS daily,
       SUM(CASE WHEN last_seen > ?2 THEN 1 ELSE 0 END) AS weekly,
       SUM(CASE WHEN last_seen > ?3 THEN 1 ELSE 0 END) AS monthly,
       SUM(CASE WHEN last_seen > ?2 AND plan = 'free' THEN 1 ELSE 0 END) AS free_weekly,
       SUM(CASE WHEN first_seen > ?2 THEN 1 ELSE 0 END) AS new_weekly,
       COUNT(*) AS total
     FROM installs WHERE ${NOT_MINE_INSTALL}`,
  )
    .bind(t - 2 * DAY, t - 8 * DAY, t - 31 * DAY, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_INSTALLS))
    .first<Usage>();
  return {
    daily: row?.daily ?? 0,
    weekly: row?.weekly ?? 0,
    monthly: row?.monthly ?? 0,
    free_weekly: row?.free_weekly ?? 0,
    new_weekly: row?.new_weekly ?? 0,
    total: row?.total ?? 0,
  };
}

/** The same installs the weekly tile counts, split by the OS each one reported
 *  on its last ping. Answers "a new person turned up — what are they running?"
 *  without naming any single machine. `os` is Rust's `env::consts::OS`, so the
 *  values are `macos` / `windows` / `linux`. */
export interface OsRow {
  os: string;
  today: number;
  active_today: number;
  active: number;
  total: number;
}

export async function osBreakdown(env: Env): Promise<OsRow[]> {
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(os, 'unknown') AS os,
            SUM(CASE WHEN first_seen >= ?2 THEN 1 ELSE 0 END) AS today,
            SUM(CASE WHEN last_seen  >= ?2 THEN 1 ELSE 0 END) AS active_today,
            SUM(CASE WHEN last_seen  >  ?1 THEN 1 ELSE 0 END) AS active,
            COUNT(*) AS total
       FROM installs
      WHERE ${NOT_MINE_INSTALL}
      GROUP BY os`,
  )
    .bind(now() - 8 * DAY, startOfToday(), null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_INSTALLS))
    .all<OsRow>();
  return results ?? [];
}

export interface LicenceOsRow {
  os: string;
  today: number;
  live: number;
  expiring: number;
}

/** Licences by the OS of the devices they were activated on. A licence on two
 *  systems counts under both; one never activated counts as "unknown". */
export async function licenceOs(env: Env): Promise<LicenceOsRow[]> {
  const t = now();
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(s.os, 'unknown') AS os,
            COUNT(DISTINCT CASE WHEN l.created_at >= ?2 THEN l.id END) AS today,
            COUNT(DISTINCT CASE WHEN l.status != 'refunded' AND l.expires_at > ?1 THEN l.id END) AS live,
            COUNT(DISTINCT CASE WHEN l.status != 'refunded' AND l.expires_at > ?1
                                 AND l.expires_at < ?1 + 7 * ${DAY} THEN l.id END) AS expiring
       FROM licences l LEFT JOIN seats s ON s.licence_id = l.id
      WHERE (l.email IS NULL OR LOWER(l.email) NOT IN (SELECT value FROM json_each(?9)))
      GROUP BY COALESCE(s.os, 'unknown')`,
  )
    .bind(t, startOfToday(), null, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_EMAILS))
    .all<LicenceOsRow>();
  return results ?? [];
}

/** How the app names an OS to how a person reads it. An unlisted value (a
 *  phone build, a BSD) is shown as it arrived rather than hidden. */
export function osLabel(os: string): string {
  const known: Record<string, string> = {
    macos: "macOS",
    windows: "Windows",
    linux: "Linux",
    unknown: "Unknown",
  };
  return known[os] ?? os;
}

export async function stats(env: Env): Promise<Stats> {
  const t = now();
  // One pass with conditional sums rather than eight queries. "Live" here is
  // the same rule `isLive` applies, spelled in SQL: not refunded, and the term
  // has not run out.
  const row = await env.DB.prepare(
    `SELECT
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?1 THEN 1 ELSE 0 END) AS live,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?1 AND cycle = 'monthly' THEN 1 ELSE 0 END) AS monthly,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?1 AND cycle = 'yearly'  THEN 1 ELSE 0 END) AS yearly,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?1 AND provider = 'paddle' THEN 1 ELSE 0 END) AS paddle,
       SUM(CASE WHEN status  = 'cancelled' AND expires_at > ?1 THEN 1 ELSE 0 END) AS winding_down,
       SUM(CASE WHEN status != 'refunded' AND expires_at > ?1 AND expires_at < ?2 THEN 1 ELSE 0 END) AS expiring,
       SUM(CASE WHEN created_at > ?3 THEN 1 ELSE 0 END) AS fresh,
       COUNT(*) AS total
     FROM licences WHERE ${NOT_MINE_LICENCE}`,
  )
    .bind(t, t + 7 * DAY, t - 30 * DAY, null, null, null, null, null, ignoreList(env.ADMIN_IGNORE_EMAILS))
    .first<Stats>();

  // SUM over an empty table is NULL, not 0.
  return {
    live: row?.live ?? 0,
    monthly: row?.monthly ?? 0,
    yearly: row?.yearly ?? 0,
    paddle: row?.paddle ?? 0,
    winding_down: row?.winding_down ?? 0,
    expiring: row?.expiring ?? 0,
    fresh: row?.fresh ?? 0,
    total: row?.total ?? 0,
  };
}


export interface Row extends Licence {
  created_at: number;
  seats: number;
}

export const SELECT_ROWS = `
  SELECT l.id, l.plan, l.cycle, l.translation_limit, l.status, l.seat_limit,
         l.expires_at, l.renews_at, l.email, l.provider, l.provider_ref,
         l.created_at,
         (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id) AS seats
    FROM licences l`;

/** Finds licences by whatever the operator pasted into the box.
 *
 *  Three shapes, because support arrives in three shapes: a licence key from
 *  the customer's email, a licence id from a log line, or — most often — just
 *  the address they wrote from. The key is hashed before it is looked up,
 *  since that is the only form the database holds. */
export async function search(env: Env, query: string): Promise<Row[]> {
  const term = query.trim();
  if (!term) {
    const { results } = await env.DB.prepare(
      `${SELECT_ROWS} ORDER BY l.created_at DESC LIMIT ?`,
    )
      .bind(PAGE_SIZE)
      .all<Row>();
    return results ?? [];
  }

  if (/^BT-/i.test(term)) {
    const licence = await env.DB.prepare(`${SELECT_ROWS} WHERE l.key_hash = ?`)
      .bind(await sha256Hex(term.toUpperCase()))
      .first<Row>();
    return licence ? [licence] : [];
  }

  if (/^lc_/.test(term)) {
    const licence = await env.DB.prepare(`${SELECT_ROWS} WHERE l.id = ?`)
      .bind(term)
      .first<Row>();
    return licence ? [licence] : [];
  }

  const { results } = await env.DB.prepare(
    `${SELECT_ROWS} WHERE l.email LIKE ? OR l.provider_ref = ? ORDER BY l.created_at DESC LIMIT ?`,
  )
    .bind(`%${term}%`, term, PAGE_SIZE)
    .all<Row>();
  return results ?? [];
}

/** Recent checkouts that did not become licences. Worth a glance: a run of
 *  these is a broken price, a wrong credential or a processor outage, and none
 *  of those announce themselves anywhere else. */
export async function recentFailures(env: Env) {
  const { results } = await env.DB.prepare(
    `SELECT ref, provider, cycle, email, failure, created_at
       FROM orders WHERE status = 'failed' AND created_at > ?
       ORDER BY created_at DESC LIMIT 10`,
  )
    .bind(now() - 7 * DAY)
    .all<{
      ref: string;
      provider: string;
      cycle: string;
      email: string | null;
      failure: string | null;
      created_at: number;
    }>();
  return results ?? [];
}
