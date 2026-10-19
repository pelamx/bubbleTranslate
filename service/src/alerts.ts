// A morning email when something needs looking at, rather than waiting for
// the panel to be opened.
//
// Run daily by the cron. Each check looks at the last finished day and names
// one thing that is wrong in a sentence; a kind is mailed at most once a day,
// so a re-run cron or a second isolate never sends the same alert twice. With
// no mailer configured the alerts go to the log, like the weekly report.

import { DEEPL_DAILY_CHARS } from "./deepl";
import type { Env } from "./env";
import { deeplAccount } from "./admin/deepl";

export interface Alert {
  kind: string;
  text: string;
}

const iso = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

/** Everything wrong with yesterday (UTC), and with the last day of webhooks
 *  and checkouts. Thresholds sit above the noise of a handful of users: an
 *  alert that fires every morning is one nobody reads. */
export async function findAlerts(env: Env, t: number): Promise<Alert[]> {
  const yesterday = iso(t - 86_400);
  const alerts: Alert[] = [];

  // The free fallback answering a real share means Google is refusing: when
  // it is healthy, MyMemory wins almost nothing.
  const providers = await env.DB.prepare(
    "SELECT provider, ok, failed FROM provider_health WHERE day = ?",
  )
    .bind(yesterday)
    .all<{ provider: string; ok: number; failed: number }>();
  const rows = providers.results ?? [];
  const total = rows.reduce((a, r) => a + r.ok, 0);
  const mymemory = rows.find((r) => r.provider === "MyMemory")?.ok ?? 0;
  const refused = rows.find((r) => r.provider === "Google")?.failed ?? 0;
  if (total >= 50 && mymemory / total > 0.1) {
    alerts.push({
      kind: "fallback",
      text: `Dün çevirilerin %${Math.round((mymemory / total) * 100)} kadarını MyMemory verdi (${mymemory}/${total}). Google reddediyor olabilir.`,
    });
  }
  if (refused >= 20) {
    alerts.push({
      kind: "all-refused",
      text: `Dün ${refused} seçim hiçbir çeviriciden yanıt alamadı.`,
    });
  }

  // Paddle deliveries that went wrong: a refused or unsigned webhook is a sale
  // or a renewal that may not have reached its licence.
  const hooks = await env.DB.prepare(
    `SELECT COUNT(*) AS n FROM webhook_events
      WHERE at > ? AND outcome IN ('refused', 'bad signature', 'error')`,
  )
    .bind(t - 86_400)
    .first<{ n: number }>();
  if ((hooks?.n ?? 0) > 0) {
    alerts.push({
      kind: "webhooks",
      text: `Son 24 saatte ${hooks?.n} Paddle webhook'u hatalı sonuçlandı. Panelde Payments → Webhooks'a bak.`,
    });
  }

  const failedCheckouts = await env.DB.prepare(
    "SELECT COUNT(*) AS n FROM orders WHERE status = 'failed' AND created_at > ?",
  )
    .bind(t - 86_400)
    .first<{ n: number }>();
  if ((failedCheckouts?.n ?? 0) >= 2) {
    alerts.push({
      kind: "checkouts",
      text: `Son 24 saatte ${failedCheckouts?.n} ödeme başarısız oldu.`,
    });
  }

  // Claude failing for people with a key: a changed model name, a broken
  // request, or Anthropic down.
  const claude = await env.DB.prepare(
    "SELECT SUM(requests) AS ok, SUM(failed) AS failed FROM claude_usage WHERE day = ?",
  )
    .bind(yesterday)
    .first<{ ok: number | null; failed: number | null }>()
    .catch(() => null);
  const claudeOk = claude?.ok ?? 0;
  const claudeFailed = claude?.failed ?? 0;
  if (claudeOk + claudeFailed >= 20 && claudeFailed / (claudeOk + claudeFailed) > 0.2) {
    alerts.push({
      kind: "claude",
      text: `Dün Claude isteklerinin %${Math.round((claudeFailed / (claudeOk + claudeFailed)) * 100)} kadarı başarısız oldu (${claudeFailed}/${claudeOk + claudeFailed}).`,
    });
  }

  // The DeepL fallback: a day at its cap, or a month nearly spent.
  const deepl = await env.DB.prepare("SELECT SUM(chars) AS chars FROM deepl_usage WHERE day = ?")
    .bind(yesterday)
    .first<{ chars: number | null }>();
  if ((deepl?.chars ?? 0) >= DEEPL_DAILY_CHARS * 0.95) {
    alerts.push({
      kind: "deepl-day",
      text: `Dün DeepL yedeği günlük sınırına ulaştı (${deepl?.chars} / ${DEEPL_DAILY_CHARS} karakter).`,
    });
  }
  const month = await deeplAccount(env);
  if (typeof month !== "string" && month.limit > 0 && month.used / month.limit > 0.8) {
    alerts.push({
      kind: "deepl-month",
      text: `DeepL anahtarının bu ayki kotasının %${Math.round((month.used / month.limit) * 100)} kadarı kullanıldı.`,
    });
  }

  return alerts;
}

/** Keeps only the alerts not yet sent today, and marks them sent. */
async function claim(env: Env, alerts: Alert[], t: number): Promise<Alert[]> {
  const day = iso(t);
  const fresh: Alert[] = [];
  for (const a of alerts) {
    const { meta } = await env.DB.prepare("INSERT OR IGNORE INTO alerts_sent (kind, day, at) VALUES (?, ?, ?)")
      .bind(a.kind, day, t)
      .run();
    if ((meta?.changes ?? 0) > 0) fresh.push(a);
  }
  return fresh;
}

const alertTo = (env: Env) => env.REPORT_EMAIL || env.SUPPORT_EMAIL || "support@bubbletranslate.app";

/** The cron's entry. Best-effort: a failure is logged, never thrown. */
export async function sendAlerts(env: Env): Promise<void> {
  const t = Math.floor(Date.now() / 1000);
  try {
    const alerts = await claim(env, await findAlerts(env, t), t);
    if (alerts.length === 0) return;
    const text = ["bubbleTranslate — dikkat edilmesi gerekenler", "", ...alerts.map((a) => `• ${a.text}`)].join("\n");
    if (!env.RESEND_API_KEY || !env.MAIL_FROM) {
      console.log(text);
      return;
    }
    const res = await fetch("https://api.resend.com/emails", {
      method: "POST",
      headers: { authorization: `Bearer ${env.RESEND_API_KEY}`, "content-type": "application/json" },
      body: JSON.stringify({
        from: env.MAIL_FROM,
        to: alertTo(env),
        subject: `bubbleTranslate uyarı: ${alerts[0].text.slice(0, 60)}${alerts.length > 1 ? ` (+${alerts.length - 1})` : ""}`,
        text,
      }),
    });
    if (!res.ok) {
      // Unclaimed again, so the next run tries to send them rather than
      // believing they went.
      await env.DB.batch(
        alerts.map((a) => env.DB.prepare("DELETE FROM alerts_sent WHERE kind = ? AND day = ?").bind(a.kind, iso(t))),
      );
      throw new Error(`resend ${res.status}: ${await res.text()}`);
    }
    console.log(`sent ${alerts.length} alert(s) to ${alertTo(env)}`);
  } catch (err) {
    console.error("alerts failed", err);
  }
}
