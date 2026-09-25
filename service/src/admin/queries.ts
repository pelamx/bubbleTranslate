// What the panel reads: counts, installs, versions, downloads and licences.

import { type Env, USD_AMOUNT } from "../env";
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

/** Must match `INSTALL_SALT` in the client's `src/license.rs`. */
const INSTALL_SALT = "bubbleTranslate/install/v1";

/** The installs to leave out of every count, as a JSON array for `?9`: the
 *  ones named in `ADMIN_IGNORE_INSTALLS`, plus every machine a licence marked
 *  "This is me" was activated on.
 *
 *  A seat stores the device id and the ping sends a different hash of it, on
 *  purpose, so the two cannot be joined for a customer. The join is made here
 *  only for the operator's own licences, where there is nobody's privacy to
 *  keep: the app derives its install id as the first 16 bytes of
 *  SHA-256(INSTALL_SALT + device), and so does this. */
export async function mineInstalls(env: Env): Promise<string> {
  const listed: string[] = JSON.parse(ignoreList(env.ADMIN_IGNORE_INSTALLS));
  const { results } = await env.DB.prepare(
    "SELECT DISTINCT device FROM seats WHERE licence_id IN (SELECT licence_id FROM ignored_licences)",
  ).all<{ device: string }>();
  const derived = await Promise.all(
    (results ?? []).map(async (r) => (await sha256Hex(INSTALL_SALT + r.device)).slice(0, 32)),
  );
  return JSON.stringify([...new Set([...listed, ...derived])]);
}

/** Keeps the operator's own machines and licences out of the counts. */
export const NOT_MINE_INSTALL = `install NOT IN (SELECT value FROM json_each(?9))
  AND install NOT IN (SELECT install FROM ignored_installs)`;
export const NOT_MINE_LICENCE = `(email IS NULL OR LOWER(email) NOT IN (SELECT value FROM json_each(?9)))
  AND id NOT IN (SELECT licence_id FROM ignored_licences)`;

/** The other side of {@link NOT_MINE_INSTALL}: the operator's own machines and
 *  nobody else's. Written as the exact complement so a machine can never fall
 *  through both conditions, or be caught by both. */
export const MINE_INSTALL = `(install IN (SELECT value FROM json_each(?9))
  OR install IN (SELECT install FROM ignored_installs))`;

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
    .bind(t, null, null, null, null, null, null, null, await mineInstalls(env))
    .all<HealthRow>();
  return results ?? [];
}

/** Which version each install in use this month is on, by OS. */
export type ProviderHealthRow = {
  provider: string;
  ok: number;
  failed: number;
  reports: number;
};

/// How each translation backend has fared over the last week, newest day first.
///
/// This is the answer to the one thing the app cannot tell us by itself: the
/// backend it leads with is an undocumented endpoint, and a day when it starts
/// refusing for everybody would otherwise be indistinguishable from a quiet day.
/// The tell is not the failure count -- it is the fallbacks winning anything at
/// all, because normally they win nothing.
export async function providerHealth(env: Env): Promise<ProviderHealthRow[]> {
  const { results } = await env.DB.prepare(
    `SELECT provider,
            SUM(ok)      AS ok,
            SUM(failed)  AS failed,
            SUM(reports) AS reports
       FROM provider_health
      WHERE day >= date('now', '-7 day')
   GROUP BY provider
   ORDER BY ok DESC, provider`,
  ).all<ProviderHealthRow>();
  return results ?? [];
}

/** Installs per version and OS over the last 30 days. `n` counts real users
 *  only; `mine` counts the operator's own machines beside it, so a version
 *  only they run yet still gets a row without being counted as adoption. */
export async function versions(
  env: Env,
): Promise<{ app: string; os: string; n: number; mine: number }[]> {
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(app, '?') AS app, COALESCE(os, 'unknown') AS os,
            SUM(CASE WHEN ${NOT_MINE_INSTALL} THEN 1 ELSE 0 END) AS n,
            SUM(CASE WHEN ${NOT_MINE_INSTALL} THEN 0 ELSE 1 END) AS mine
       FROM installs WHERE last_seen > ?1
      GROUP BY app, os`,
  )
    .bind(now() - 30 * DAY, null, null, null, null, null, null, null, await mineInstalls(env))
    .all<{ app: string; os: string; n: number; mine: number }>();
  return results ?? [];
}

export interface CountryRow {
  country: string;
  n: number;
  week: number;
}

/** Real installs seen in the last 30 days, by the country their pings came
 *  from, with how many of them were seen in the last eight days. Installs
 *  that have not pinged since the country was first recorded are `?`. */
export async function countries(env: Env): Promise<CountryRow[]> {
  const t = now();
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(country, '?') AS country, COUNT(*) AS n,
            SUM(CASE WHEN last_seen > ?2 THEN 1 ELSE 0 END) AS week
       FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}
      GROUP BY COALESCE(country, '?')
      ORDER BY n DESC, country`,
  )
    .bind(t - 30 * DAY, t - 8 * DAY, null, null, null, null, null, null, await mineInstalls(env))
    .all<CountryRow>();
  return results ?? [];
}

export interface CappedRow {
  install: string;
  os: string | null;
  app: string | null;
  plan: string | null;
  country: string | null;
  first_seen: number;
  first_capped: number;
  last_capped: number;
  capped_days: number;
}

export interface LimitHits {
  /** Real installs that could have reported it: first seen since reporting began. */
  base: number;
  capped: CappedRow[];
}

/** Real installs that have run out of the free allowance, most days first.
 *  The base is installs on a build that reports it at all: anything seen
 *  since the first capped report arrived, which is the best available stand-in
 *  for "running 0.3.7 or newer" without a version compare in SQL. */
export async function limitHits(env: Env): Promise<LimitHits> {
  const mine = await mineInstalls(env);
  const { results } = await env.DB.prepare(
    `SELECT install, os, app, plan, country, first_seen, first_capped, last_capped, capped_days
       FROM installs WHERE first_capped IS NOT NULL AND ${NOT_MINE_INSTALL}
      ORDER BY capped_days DESC, last_capped DESC`,
  )
    .bind(null, null, null, null, null, null, null, null, mine)
    .all<CappedRow>();
  const capped = results ?? [];
  const since = capped.reduce((a, r) => Math.min(a, r.first_capped), Infinity);
  const base = Number.isFinite(since)
    ? ((
        await env.DB.prepare(
          `SELECT COUNT(*) AS n FROM installs WHERE last_seen >= ?1 AND ${NOT_MINE_INSTALL}`,
        )
          .bind(since, null, null, null, null, null, null, null, mine)
          .first<{ n: number }>()
      )?.n ?? 0)
    : 0;
  return { base, capped };
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

/** One release's downloads, per OS. */
export interface ReleaseRow {
  tag: string;
  published: string;
  windows: number;
  linux: number;
  macos: number;
}

export interface Downloads {
  /** Every release file ever, summed per OS. */
  total: Record<string, number>;
  /** Newest release first. */
  releases: ReleaseRow[];
}

/** Every release asset's download count on GitHub, per OS, in total and per
 *  release. Cached for fifteen minutes: the API allows sixty unauthenticated
 *  calls an hour, and a Worker's outbound address is shared. Null when GitHub
 *  cannot be reached, so the panel says so rather than showing zeros. */
export async function downloads(env: Env): Promise<Downloads | null> {
  const url = "https://api.github.com/repos/pelamx/downloads/releases?per_page=100";
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
  const releases = (await res.json()) as {
    tag_name: string;
    published_at: string | null;
    assets: { name: string; download_count: number }[];
  }[];
  const total: Record<string, number> = { windows: 0, linux: 0, macos: 0 };
  const rows: ReleaseRow[] = [];
  for (const r of releases) {
    const row: ReleaseRow = {
      tag: r.tag_name,
      published: (r.published_at ?? "").slice(0, 10),
      windows: 0,
      linux: 0,
      macos: 0,
    };
    for (const a of r.assets) {
      const name = a.name.toLowerCase();
      if (name.endsWith(".zip") || name.endsWith(".exe")) row.windows += a.download_count;
      else if (name.endsWith(".dmg")) row.macos += a.download_count;
      else if (name.includes("linux")) row.linux += a.download_count;
    }
    for (const os of PLATFORMS) total[os] += row[os];
    rows.push(row);
  }
  rows.sort((a, b) => compareVersions(a.tag.replace(/^v/, ""), b.tag.replace(/^v/, "")));
  return { total, releases: rows };
}

/** The version each platform is offered right now, as `latest.json` in the
 *  downloads repository says -- the same file installed copies read, so this
 *  is what "latest" means to them. Null when it cannot be read. */
export async function published(): Promise<Record<string, string> | null> {
  const url = "https://raw.githubusercontent.com/pelamx/downloads/main/latest.json";
  try {
    // Short-lived: a release should show up here within a minute or two.
    const res = await fetch(url, { cf: { cacheTtl: 60 } } as RequestInit);
    if (!res.ok) return null;
    const manifest = (await res.json()) as Record<string, { version?: string }>;
    const out: Record<string, string> = {};
    for (const os of PLATFORMS) {
      const v = manifest[os]?.version;
      if (typeof v === "string") out[os] = v;
    }
    return out;
  } catch {
    return null;
  }
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
    .bind(now() - 30 * DAY, null, null, null, null, null, null, null, await mineInstalls(env))
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
    .bind(t - 2 * DAY, t - 8 * DAY, t - 31 * DAY, null, null, null, null, null, await mineInstalls(env))
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

/** The operator's own machines, split by OS.
 *
 *  Every other count on this page leaves these out, which is right for reading
 *  how the app is doing and wrong for the one question the operator asks about
 *  their own machines: did the install I just made arrive? Without this the
 *  answer to "I installed it and nothing shows" is indistinguishable from a
 *  ping that never happened.
 *
 *  Same shape as {@link osBreakdown} so the page can draw both with one
 *  helper, and the same `os` values — Rust's `env::consts::OS`. */
export async function mineBreakdown(env: Env): Promise<OsRow[]> {
  const { results } = await env.DB.prepare(
    `SELECT COALESCE(os, 'unknown') AS os,
            SUM(CASE WHEN first_seen >= ?2 THEN 1 ELSE 0 END) AS today,
            SUM(CASE WHEN last_seen  >= ?2 THEN 1 ELSE 0 END) AS active_today,
            SUM(CASE WHEN last_seen  >  ?1 THEN 1 ELSE 0 END) AS active,
            COUNT(*) AS total
       FROM installs
      WHERE ${MINE_INSTALL}
      GROUP BY os`,
  )
    .bind(now() - 8 * DAY, startOfToday(), null, null, null, null, null, null, await mineInstalls(env))
    .all<OsRow>();
  return results ?? [];
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
    .bind(now() - 8 * DAY, startOfToday(), null, null, null, null, null, null, await mineInstalls(env))
    .all<OsRow>();
  return results ?? [];
}

export interface LicenceOsRow {
  os: string;
  today: number;
  live: number;
  expiring: number;
}

/** Paid licences by the OS of the devices they were activated on. A licence on two
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
      WHERE l.provider = 'paddle'
        AND (l.email IS NULL OR LOWER(l.email) NOT IN (SELECT value FROM json_each(?9)))
        AND l.id NOT IN (SELECT licence_id FROM ignored_licences)
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

/** The paid side only: every figure here is read as customers or money, and a
 *  key issued by hand is neither. Manual licences are listed on their own. */
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
     FROM licences WHERE provider = 'paddle' AND ${NOT_MINE_LICENCE}`,
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
  /** 1 when the operator marked it as their own. */
  mine: number;
}

export const SELECT_ROWS = `
  SELECT l.id, l.plan, l.cycle, l.translation_limit, l.status, l.seat_limit,
         l.expires_at, l.renews_at, l.email, l.provider, l.provider_ref,
         l.created_at,
         (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id) AS seats,
         EXISTS (SELECT 1 FROM ignored_licences i WHERE i.licence_id = l.id) AS mine
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
    // A page of each kind rather than one page of both: the panel shows them
    // apart, and a run of hand-issued keys must not push every sale off it.
    const latest = (where: string) =>
      env.DB.prepare(`${SELECT_ROWS} WHERE ${where} ORDER BY l.created_at DESC LIMIT ?`)
        .bind(PAGE_SIZE)
        .all<Row>()
        .then((r) => r.results ?? []);
    const [paid, manual] = await Promise.all([
      latest("l.provider = 'paddle'"),
      latest("l.provider != 'paddle'"),
    ]);
    return [...paid, ...manual];
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

// -- history -----------------------------------------------------------------

export interface AdminLogRow {
  at: number;
  action: string;
  licence_id: string | null;
  detail: string | null;
}

/** The operator's own actions, newest first. */
export async function adminLog(env: Env, limit = 30): Promise<AdminLogRow[]> {
  const { results } = await env.DB.prepare(
    "SELECT at, action, licence_id, detail FROM admin_log ORDER BY id DESC LIMIT ?",
  )
    .bind(limit)
    .all<AdminLogRow>();
  return results ?? [];
}

export interface WebhookRow {
  at: number;
  event_type: string | null;
  event_id: string | null;
  entity_id: string | null;
  outcome: string;
  detail: string | null;
}

/** The latest deliveries from Paddle, and how many of the last week's went
 *  wrong -- refused, unsigned or failing. Ignored is not wrong: Paddle sends
 *  events nothing here needs. */
export async function webhookEvents(env: Env): Promise<{ rows: WebhookRow[]; bad: number }> {
  const [list, bad] = await Promise.all([
    env.DB.prepare(
      `SELECT at, event_type, event_id, entity_id, outcome, detail
         FROM webhook_events ORDER BY id DESC LIMIT 30`,
    ).all<WebhookRow>(),
    env.DB.prepare(
      `SELECT COUNT(*) AS n FROM webhook_events
        WHERE at > ? AND outcome IN ('refused', 'bad signature', 'error')`,
    )
      .bind(now() - 7 * DAY)
      .first<{ n: number }>(),
  ]);
  return { rows: list.results ?? [], bad: bad?.n ?? 0 };
}

// -- money -------------------------------------------------------------------

export interface EndingRow extends Row {
  /** Paddle's status for the subscription behind it, if a webhook said. */
  sub_status: string | null;
  scheduled_change_action: string | null;
}

/** Live licences whose term ends in the next 30 days, soonest first, each with
 *  what the mirror knows about whether Paddle will renew it. */
export async function endingSoon(env: Env): Promise<EndingRow[]> {
  const t = now();
  const { results } = await env.DB.prepare(
    `SELECT l.id, l.plan, l.cycle, l.translation_limit, l.status, l.seat_limit,
            l.expires_at, l.renews_at, l.email, l.provider, l.provider_ref, l.created_at,
            (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id) AS seats,
            EXISTS (SELECT 1 FROM ignored_licences i WHERE i.licence_id = l.id) AS mine,
            ps.status AS sub_status, ps.scheduled_change_action
       FROM licences l
       LEFT JOIN paddle_subscriptions ps ON ps.subscription_id = l.provider_ref
      WHERE l.status != 'refunded' AND l.expires_at > ?1 AND l.expires_at < ?1 + 30 * ${DAY}
      ORDER BY l.expires_at
      LIMIT 100`,
  )
    .bind(t)
    .all<EndingRow>();
  return results ?? [];
}

/** Whether a licence ending soon is going to be renewed: yes for a Paddle
 *  subscription that is active with nothing scheduled, no for one cancelled
 *  or issued by hand, and unknown when no webhook has described it. */
export function renews(r: EndingRow): "yes" | "no" | "unknown" {
  if (r.status === "cancelled" || r.provider !== "paddle") return "no";
  if (!r.sub_status) return "unknown";
  if (r.sub_status === "canceled" || r.scheduled_change_action === "cancel") return "no";
  return r.sub_status === "active" || r.sub_status === "trialing" ? "yes" : "unknown";
}

export interface MonthRow {
  month: string;
  new_monthly: number;
  new_yearly: number;
  cancelled: number;
  refunded: number;
}

export interface Revenue {
  /** Monthly recurring revenue at list price, from subscriptions set to renew. */
  mrr: number;
  renewing_monthly: number;
  renewing_yearly: number;
  /** Newest month first, the last six. */
  months: MonthRow[];
}

/** Paid licences as money, at list price in USD -- the same figures and the
 *  same MRR rule as the weekly report. What Paddle actually collected differs
 *  by currency and tax; this is what the product charges, not what it banked. */
export async function revenue(env: Env): Promise<Revenue> {
  const t = now();
  const mine = ignoreList(env.ADMIN_IGNORE_EMAILS);
  // Six calendar months back, as UTC "YYYY-MM" labels.
  const d = new Date(t * 1000);
  const labels = Array.from({ length: 6 }, (_, i) =>
    new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() - i, 1)).toISOString().slice(0, 7),
  );
  const since = Math.floor(Date.parse(`${labels[5]}-01T00:00:00Z`) / 1000);

  const [renewing, fresh, cancelled, refunded] = await Promise.all([
    env.DB.prepare(
      `SELECT cycle, COUNT(*) AS n FROM licences
        WHERE provider = 'paddle' AND status = 'active' AND expires_at > ?1 AND ${NOT_MINE_LICENCE}
        GROUP BY cycle`,
    )
      .bind(t, null, null, null, null, null, null, null, mine)
      .all<{ cycle: string; n: number }>(),
    env.DB.prepare(
      `SELECT strftime('%Y-%m', created_at, 'unixepoch') AS month, cycle, COUNT(*) AS n
         FROM licences
        WHERE provider = 'paddle' AND created_at >= ?1 AND ${NOT_MINE_LICENCE}
        GROUP BY month, cycle`,
    )
      .bind(since, null, null, null, null, null, null, null, mine)
      .all<{ month: string; cycle: string; n: number }>(),
    // When a subscription was cancelled is when the mirror last heard it was:
    // the licence row keeps no date for it.
    env.DB.prepare(
      `SELECT strftime('%Y-%m', ps.updated_at, 'unixepoch') AS month, COUNT(*) AS n
         FROM paddle_subscriptions ps JOIN licences l ON l.provider_ref = ps.subscription_id
        WHERE ps.status = 'canceled' AND ps.updated_at >= ?1
          AND (l.email IS NULL OR LOWER(l.email) NOT IN (SELECT value FROM json_each(?9)))
          AND l.id NOT IN (SELECT licence_id FROM ignored_licences)
        GROUP BY month`,
    )
      .bind(since, null, null, null, null, null, null, null, mine)
      .all<{ month: string; n: number }>(),
    // A refund cuts the term to the moment it happened, so its end is its date.
    env.DB.prepare(
      `SELECT strftime('%Y-%m', expires_at, 'unixepoch') AS month, COUNT(*) AS n
         FROM licences
        WHERE provider = 'paddle' AND status = 'refunded' AND expires_at >= ?1 AND ${NOT_MINE_LICENCE}
        GROUP BY month`,
    )
      .bind(since, null, null, null, null, null, null, null, mine)
      .all<{ month: string; n: number }>(),
  ]);

  const count = (cycle: string) =>
    (renewing.results ?? []).filter((r) => r.cycle === cycle).reduce((a, r) => a + r.n, 0);
  const renewing_monthly = count("monthly");
  const renewing_yearly = count("yearly");

  const months = labels.map((month) => ({
    month,
    new_monthly: (fresh.results ?? []).find((r) => r.month === month && r.cycle === "monthly")?.n ?? 0,
    new_yearly: (fresh.results ?? []).find((r) => r.month === month && r.cycle === "yearly")?.n ?? 0,
    cancelled: (cancelled.results ?? []).find((r) => r.month === month)?.n ?? 0,
    refunded: (refunded.results ?? []).find((r) => r.month === month)?.n ?? 0,
  }));

  return {
    mrr: renewing_monthly * USD_AMOUNT.monthly + renewing_yearly * (USD_AMOUNT.yearly / 12),
    renewing_monthly,
    renewing_yearly,
    months,
  };
}

/** How many of this week's active installs run Pro -- the conversion the
 *  ping can show without keeping anything more than it already does. */
export async function proShare(env: Env): Promise<{ pro: number; active: number }> {
  const row = await env.DB.prepare(
    `SELECT SUM(CASE WHEN plan = 'pro' THEN 1 ELSE 0 END) AS pro, COUNT(*) AS active
       FROM installs WHERE last_seen > ?1 AND ${NOT_MINE_INSTALL}`,
  )
    .bind(now() - 8 * DAY, null, null, null, null, null, null, null, await mineInstalls(env))
    .first<{ pro: number | null; active: number }>();
  return { pro: row?.pro ?? 0, active: row?.active ?? 0 };
}
