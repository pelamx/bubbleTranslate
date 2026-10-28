import type { Env } from "./env";
import { escapeHtml } from "./pages";
import { mineInstalls } from "./admin/queries";

const FEATURES = new Set(["app", "manual", "selection", "clipboard", "ocr_box", "ocr_page", "retranslate", "speech", "copy"]);
const OUTCOMES = new Set(["started", "panic", "capture_not_ready", "monitor_failed", "attempt", "capped", "ok", "echoed", "translation_failed", "capture_empty", "cancelled_or_unavailable", "capture_failed", "ocr_empty", "no_translated_blocks", "filtered", "renderer_failed", "used", "start_failed", "partial", "rate_limited", "network_failed", "bad_response", "provider_unavailable", "http_failed"]);

/** Cumulative daily snapshots: an old, duplicate or reordered delivery cannot add usage. */
export async function recordMetrics(env: Env, install: string, os: string, channel: string, input: unknown, t: number): Promise<boolean> {
  if (!Array.isArray(input)) return true;
  const today = Math.floor(t / 86400);
  const statements: D1PreparedStatement[] = [];
  for (const row of input.slice(0, 4096)) {
    if (!row || !Number.isInteger(row.day) || row.day <= today - 32 || row.day > today + 1 ||
      !FEATURES.has(row.feature) || !OUTCOMES.has(row.outcome) ||
      typeof row.app !== "string" || !/^\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?$/.test(row.app) || row.app.length > 32 ||
      !Number.isSafeInteger(row.count) || row.count <= 0) continue;
    const rowChannel = row.channel === "store" || row.channel === "download" ? row.channel : channel;
    const day = new Date(row.day * 86400000).toISOString().slice(0, 10);
    statements.push(env.DB.prepare(
      `INSERT INTO feature_metrics (day, install, app, os, channel, feature, outcome, count)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT (day, install, app, os, channel, feature, outcome)
       DO UPDATE SET count = excluded.count WHERE excluded.count > feature_metrics.count`,
    ).bind(day, install, row.app, os, rowChannel, row.feature, row.outcome, Math.min(row.count, 1000000)));
  }
  try {
    // D1 accepts a limited number of bound parameters per statement, not per batch.
    for (let i = 0; i < statements.length; i += 100) await env.DB.batch(statements.slice(i, i + 100));
    await env.DB.prepare("DELETE FROM feature_metrics WHERE day < ?")
      .bind(new Date((t - 90 * 86400) * 1000).toISOString().slice(0, 10)).run();
    return true;
  } catch (err) {
    console.error("could not record feature metrics", err);
    return false;
  }
}

export interface RemoteSettings {
  debounce_ms?: number;
  translation_timeout_ms?: number;
  disabled_providers?: string[];
  dictionary_enabled?: boolean;
  update_message?: string;
  update_before_version?: string;
  /** Sets the free plan's translations a day, 3 to 10. Only copies that
   *  send their version are given it -- see {@link remoteSettings}. */
  free_daily_translations?: number;
}

/** The free plan's translations a day the app builds in. */
export const FREE_DAILY_TRANSLATIONS = 3;
export function validateSettings(input: unknown): RemoteSettings | null {
  if (!input || typeof input !== "object" || Array.isArray(input)) return null;
  const s = input as Record<string, unknown>;
  if (Object.keys(s).some(k => !["debounce_ms", "translation_timeout_ms", "disabled_providers", "dictionary_enabled", "update_message", "update_before_version", "free_daily_translations"].includes(k))) return null;
  if (s.debounce_ms !== undefined && (!Number.isInteger(s.debounce_ms) || Number(s.debounce_ms) < 0 || Number(s.debounce_ms) > 2000)) return null;
  if (s.translation_timeout_ms !== undefined && (!Number.isInteger(s.translation_timeout_ms) || Number(s.translation_timeout_ms) < 2000 || Number(s.translation_timeout_ms) > 30000)) return null;
  if (s.disabled_providers !== undefined && (!Array.isArray(s.disabled_providers) || s.disabled_providers.length > 4 || s.disabled_providers.some(p => !["google", "mymemory", "deepl", "claude"].includes(p)))) return null;
  if (s.dictionary_enabled !== undefined && typeof s.dictionary_enabled !== "boolean") return null;
  if (s.update_message !== undefined && (typeof s.update_message !== "string" || [...s.update_message].length > 300)) return null;
  if (s.update_before_version !== undefined && (typeof s.update_before_version !== "string" || !/^\d{1,9}\.\d{1,9}\.\d{1,9}$/.test(s.update_before_version))) return null;
  if (s.free_daily_translations !== undefined && (!Number.isInteger(s.free_daily_translations) || Number(s.free_daily_translations) < 3 || Number(s.free_daily_translations) > 10)) return null;
  return s as RemoteSettings;
}

/** What a copy on `os` and `channel` is told. `app` is the version a copy
 *  sends from 0.4.23 on; the app reads its settings with `deny_unknown_fields`,
 *  so an older copy given a control it does not know would throw the whole
 *  answer away, emergency switches and all. Newer controls go only to copies
 *  that say which version they are. */
export async function remoteSettings(env: Env, os: string, channel: string, app: string | null = null): Promise<RemoteSettings> {
  const scopes = ["*:*", `${os}:*`, `*:${channel}`, `${os}:${channel}`];
  const { results } = await env.DB.prepare("SELECT scope, settings FROM remote_settings WHERE scope IN (?, ?, ?, ?)")
    .bind(...scopes).all<{scope: string; settings: string}>();
  const combined: RemoteSettings = {};
  for (const scope of scopes) {
    const row = results.find(r => r.scope === scope);
    if (row) {
      try { Object.assign(combined, validateSettings(JSON.parse(row.settings)) ?? {}); } catch { /* use defaults */ }
    }
  }
  if (!app) delete combined.free_daily_translations;
  return combined;
}

/** The free plan's translations a day as every platform is told them: the
 *  all-platforms override, or what the app builds in. What the buy page and
 *  the panel say, so neither promises more than the bubble allows. A
 *  platform-only override is not reflected here. */
export async function freeDailyTranslations(env: Env): Promise<number> {
  try {
    const row = await env.DB.prepare("SELECT settings FROM remote_settings WHERE scope = '*:*'").first<{ settings: string }>();
    const n = row ? validateSettings(JSON.parse(row.settings))?.free_daily_translations : undefined;
    return n ?? FREE_DAILY_TRANSLATIONS;
  } catch {
    return FREE_DAILY_TRANSLATIONS;
  }
}

export async function saveRemoteSettings(env: Env, request: Request): Promise<Response> {
  const form = await request.formData();
  const os = String(form.get("os") ?? "*");
  const channel = String(form.get("channel") ?? "*");
  if (!["*", "linux", "windows", "macos"].includes(os) || !["*", "store", "download"].includes(channel)) return new Response("Invalid scope", {status: 400});
  let settings: RemoteSettings | null;
  const raw = String(form.get("settings") ?? "{}");
  if (raw.length > 4096) return new Response("Settings too large", {status: 400});
  try { settings = validateSettings(JSON.parse(raw)); } catch { settings = null; }
  if (!settings) return new Response("Invalid settings. Allowed: debounce_ms (0–2000), translation_timeout_ms (2000–30000), disabled_providers (provider names), dictionary_enabled (boolean), update_message (up to 300 characters), update_before_version (major.minor.patch), free_daily_translations (3–10, sets the free allowance; only 0.4.23 and later).", {status: 400});
  const scope = `${os}:${channel}`;
  const t = Math.floor(Date.now() / 1000);
  await env.DB.batch([
    env.DB.prepare("INSERT INTO remote_settings (scope, settings, updated_at) VALUES (?, ?, ?) ON CONFLICT(scope) DO UPDATE SET settings=excluded.settings, updated_at=excluded.updated_at")
      .bind(scope, JSON.stringify(settings), t),
    env.DB.prepare("INSERT INTO admin_log (at, action, licence_id, detail) VALUES (?, 'remote-config', NULL, ?)")
      .bind(t, `${scope}: ${JSON.stringify(settings)}`),
  ]);
  return new Response(null, {status: 303, headers: {location: "/admin#remote-config"}});
}

export async function observabilityCard(env: Env): Promise<string> {
  try {
    const mine = await mineInstalls(env);
    const since = new Date(Date.now() - 29 * 86400000).toISOString().slice(0, 10);
    const [usage, errors, activation, configs, repeat, installs] = await Promise.all([
      env.DB.prepare(`SELECT feature, outcome, SUM(count) AS n, COUNT(DISTINCT install) AS people
        FROM feature_metrics WHERE day >= ? AND feature <> 'app'
        AND install NOT IN (SELECT install FROM ignored_installs)
        AND install NOT IN (SELECT value FROM json_each(?))
        GROUP BY feature, outcome ORDER BY feature, outcome`).bind(since, mine).all<any>(),
      env.DB.prepare(`SELECT app, os, feature, outcome, SUM(count) AS n, COUNT(DISTINCT install) AS people
        FROM feature_metrics WHERE day >= ? AND outcome IN ('panic','capture_not_ready','monitor_failed','translation_failed','capture_empty','capture_failed','ocr_empty','no_translated_blocks','renderer_failed','start_failed','partial','rate_limited','network_failed','bad_response','provider_unavailable','http_failed')
        AND install NOT IN (SELECT install FROM ignored_installs)
        AND install NOT IN (SELECT value FROM json_each(?))
        GROUP BY app, os, feature, outcome ORDER BY n DESC LIMIT 50`).bind(since, mine).all<any>(),
      env.DB.prepare(`SELECT COUNT(DISTINCT install) AS active,
        COUNT(DISTINCT CASE WHEN outcome = 'ok' THEN install END) AS successful
        FROM feature_metrics WHERE day >= ?
        AND install NOT IN (SELECT install FROM ignored_installs)
        AND install NOT IN (SELECT value FROM json_each(?))`).bind(since, mine).first<{active:number; successful:number}>(),
      env.DB.prepare("SELECT scope, settings FROM remote_settings ORDER BY scope").all<{scope:string;settings:string}>(),
      env.DB.prepare(`SELECT feature, COUNT(*) AS users, SUM(CASE WHEN days >= 2 THEN 1 ELSE 0 END) AS repeat_users
        FROM (SELECT feature, install, COUNT(DISTINCT day) AS days FROM feature_metrics
          WHERE day >= ? AND outcome = 'ok'
          AND install NOT IN (SELECT install FROM ignored_installs)
          AND install NOT IN (SELECT value FROM json_each(?))
          GROUP BY feature, install) GROUP BY feature ORDER BY repeat_users DESC`).bind(since, mine).all<any>(),
      env.DB.prepare(`SELECT m.install, i.app, i.os,
        SUM(CASE WHEN m.outcome = 'ok' THEN m.count ELSE 0 END) AS successful,
        SUM(CASE WHEN m.outcome = 'translation_failed' THEN m.count ELSE 0 END) AS failed
        FROM feature_metrics m LEFT JOIN installs i ON i.install = m.install
        WHERE m.day >= ? AND m.install NOT IN (SELECT install FROM ignored_installs)
        AND m.install NOT IN (SELECT value FROM json_each(?))
        GROUP BY m.install ORDER BY successful DESC LIMIT 50`).bind(since, mine).all<any>(),
    ]);
    const e = escapeHtml;
    const rows = usage.results.map(r => `<tr><td>${e(r.feature)}</td><td>${e(r.outcome)}</td><td>${r.n}</td><td>${r.people}</td></tr>`).join("");
    const repeats = repeat.results.map(r => `<tr><td>${e(r.feature)}</td><td>${r.users}</td><td>${r.repeat_users}</td></tr>`).join("");
    const installRows = installs.results.map(r => `<tr><td><code>${e(r.install)}</code></td><td>${e(r.app ?? "")} / ${e(r.os ?? "")}</td><td>${r.successful}</td><td>${r.failed}</td></tr>`).join("");
    const failures = errors.results.map(r => `<tr><td>${e(r.app)} / ${e(r.os)}</td><td>${e(r.feature)} / ${e(r.outcome)}</td><td>${r.n}</td><td>${r.people}</td></tr>`).join("");
    return `<section class="card" id="feature-analytics"><p class="label">Feature analytics, last 30 days</p>
      <p>${activation?.successful ?? 0} of ${activation?.active ?? 0} reporting installs completed a translation. Counts exclude echoed text. Older clients do not report these metrics.</p>
      <table><tr><th>Feature</th><th>Outcome</th><th>Count</th><th>Users</th></tr>${rows || '<tr><td colspan="4">Nothing reported yet.</td></tr>'}</table>
      <p class="sub">Repeat use: successful translations on at least two separate days in this window; this is not a cohort retention rate.</p>
      <table><tr><th>Feature</th><th>Successful users</th><th>Used on 2+ days</th></tr>${repeats}</table>
      <p class="sub">Top 50 reporting installs, last 30 days. Successful requests include retranslations; page OCR counts one completed screen.</p>
      <table><tr><th>Install</th><th>Current version / OS</th><th>Translations</th><th>Translation failures</th></tr>${installRows}</table></section>
      <section class="card" id="error-monitoring"><p class="label">Error monitoring, last 30 days</p>
      <p class="sub">Fixed error codes only; no text or screenshots. Empty selection can mean no text was selected. Cancelled OCR is kept separate. Reason codes describe failed requests and must not be added to translation_failed totals. Panics are reported on the next launch; native crashes are not captured.</p>
      <table><tr><th>Version / OS</th><th>Stage / outcome</th><th>Count</th><th>Users</th></tr>${failures || '<tr><td colspan="4">No errors reported.</td></tr>'}</table></section>
      <section class="card" id="remote-config"><p class="label">Remote configuration</p>
      <p class="sub">Refreshes every five minutes. Overrides expire after 24 hours without a successful refresh. Local settings stay saved. Use {} to clear a scope. Specific platform and channel overrides win.</p>
      ${configs.results.map(r => `<p><b>${e(r.scope)}</b> <code>${e(r.settings)}</code></p>`).join("")}
      <form method="post" action="/admin/remote-config"><label>Platform <select name="os"><option value="*">All</option><option>linux</option><option>windows</option><option>macos</option></select></label>
      <label>Channel <select name="channel"><option value="*">All</option><option>download</option><option>store</option></select></label>
      <p><textarea name="settings" rows="4" cols="60" maxlength="4096">{}</textarea></p>
      <p class="sub">Example: {&quot;debounce_ms&quot;:180,&quot;translation_timeout_ms&quot;:10000,&quot;disabled_providers&quot;:[&quot;mymemory&quot;],&quot;dictionary_enabled&quot;:false,&quot;update_message&quot;:&quot;Please update to the latest version.&quot;,&quot;update_before_version&quot;:&quot;0.4.22&quot;,&quot;free_daily_translations&quot;:5}</p>
      <p class="sub"><b>free_daily_translations</b> sets the free plan's translations a day (3–10; the app's own is 3). Only copies from 0.4.23 on are told it; set it for All platforms so the buy page says the same number, and change the website's prices page by hand.</p>
      <button type="submit">Save configuration</button></form></section>`;
  } catch (err) {
    console.error("could not read observability", err);
    return '<section class="card" id="feature-analytics"><p>Feature analytics requires the latest database schema.</p></section>';
  }
}
