// The weekly founder's report.
//
// One plain-language email a week: what sold, what it was worth, and how that
// compares to the week before. It reads the service's own D1 -- new paid
// orders and the current stock of live licences -- so it needs no external
// account to produce the revenue side.
//
// It deliberately does NOT know ad spend: that lives in Reddit/Google/Meta and
// needs their APIs. Rather than half-report profit, the email states revenue
// plainly and gives the break-even line ("spend less than this and you were up
// this week"), which is the one number a solo founder can act on without
// opening a dashboard.

import { Env, USD_AMOUNT } from "./env";

const WEEK = 7 * 86_400;

// Revenue is counted by multiplying the order count for each cycle by the list
// price in USD_AMOUNT (env.ts), which is also the value ads.ts reports. It is
// deliberately not a sum of what the cards were actually charged: Paddle bills
// each buyer in their own currency, so that sum would add up several
// currencies at once. One currency at list price is the comparable number.

interface Sales {
  monthly: number;
  yearly: number;
  count: number;
  revenue: number; // USD, whole dollars at list price
}

interface Metrics {
  now: number;
  sales: Sales;
  prev: Sales;
  activeMonthly: number;
  activeYearly: number;
  activeTotal: number;
  mrr: number; // USD estimate
  activeInstalls: number; // pinged in the last week
  freeInstalls: number;
  newInstalls: number; // first ping this week
}

/** Paid orders in [from, to). Renewals do not create orders, so this is new
 *  business -- exactly what a paid-ad week is trying to produce. */
async function salesIn(env: Env, from: number, to: number): Promise<Sales> {
  const rows = await env.DB.prepare(
    `SELECT cycle, COUNT(*) AS n
       FROM orders
      WHERE status = 'paid' AND created_at >= ? AND created_at < ?
        AND (licence_id IS NULL OR licence_id NOT IN (SELECT licence_id FROM ignored_licences))
      GROUP BY cycle`,
  )
    .bind(from, to)
    .all<{ cycle: string; n: number }>();
  const out: Sales = { monthly: 0, yearly: 0, count: 0, revenue: 0 };
  for (const r of rows.results ?? []) {
    if (r.cycle === "yearly") out.yearly = r.n;
    else out.monthly = r.n;
  }
  out.count = out.monthly + out.yearly;
  out.revenue = out.monthly * USD_AMOUNT.monthly + out.yearly * USD_AMOUNT.yearly;
  return out;
}

export async function gather(env: Env, now: number): Promise<Metrics> {
  const sales = await salesIn(env, now - WEEK, now);
  const prev = await salesIn(env, now - 2 * WEEK, now - WEEK);

  const active = await env.DB.prepare(
    `SELECT cycle, COUNT(*) AS n
       FROM licences
      WHERE status = 'active' AND expires_at > ? AND provider = 'paddle'
        AND id NOT IN (SELECT licence_id FROM ignored_licences)
      GROUP BY cycle`,
  )
    .bind(now)
    .all<{ cycle: string; n: number }>();
  let activeMonthly = 0;
  let activeYearly = 0;
  for (const r of active.results ?? []) {
    if (r.cycle === "yearly") activeYearly = r.n;
    else activeMonthly = r.n;
  }
  // A yearly licence is $20/12 of recurring monthly revenue; a monthly is $2.
  const mrr = activeMonthly * USD_AMOUNT.monthly + activeYearly * (USD_AMOUNT.yearly / 12);

  // "Last week" is eight days, not seven: a once-a-day ping from an app that
  // was opened at a slightly later hour than last time must still count.
  const installs = await env.DB.prepare(
    `SELECT
       SUM(CASE WHEN last_seen > ?1 THEN 1 ELSE 0 END) AS active,
       SUM(CASE WHEN last_seen > ?1 AND plan = 'free' THEN 1 ELSE 0 END) AS free,
       SUM(CASE WHEN first_seen > ?2 THEN 1 ELSE 0 END) AS fresh
     FROM installs`,
  )
    .bind(now - WEEK - 86_400, now - WEEK)
    .first<{ active: number | null; free: number | null; fresh: number | null }>();

  return {
    now,
    activeInstalls: installs?.active ?? 0,
    freeInstalls: installs?.free ?? 0,
    newInstalls: installs?.fresh ?? 0,
    sales,
    prev,
    activeMonthly,
    activeYearly,
    activeTotal: activeMonthly + activeYearly,
    mrr,
  };
}

function delta(cur: number, prev: number): string {
  const d = cur - prev;
  if (d === 0) return "geçen haftayla aynı";
  const arrow = d > 0 ? "▲" : "▼";
  return `${arrow} ${Math.abs(d)} (geçen hafta ${prev})`;
}

function money(n: number): string {
  return "$" + (Number.isInteger(n) ? String(n) : n.toFixed(2));
}

/** The report as its own little HTML page: the email body and the browser
 *  preview are the same markup, so what /admin/report shows is exactly what
 *  lands in the inbox. */
export function renderHtml(m: Metrics): string {
  const s = m.sales;
  const from = new Date((m.now - WEEK) * 1000).toISOString().slice(0, 10);
  const to = new Date(m.now * 1000).toISOString().slice(0, 10);
  const row = (label: string, value: string) =>
    `<tr><td style="padding:6px 14px 6px 0;color:#555">${label}</td>` +
    `<td style="padding:6px 0;font-weight:600">${value}</td></tr>`;

  return `<!doctype html><html><body style="margin:0;background:#f6f7f9;font:15px/1.5 -apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1a1a1a">
  <div style="max-width:560px;margin:0 auto;padding:24px">
    <div style="background:#fff;border:1px solid #e6e8eb;border-radius:12px;padding:24px">
      <h1 style="margin:0 0 4px;font-size:20px">bubbleTranslate — haftalık özet</h1>
      <p style="margin:0 0 18px;color:#777;font-size:13px">${from} → ${to}</p>

      <h2 style="font-size:15px;margin:0 0 8px">Bu hafta</h2>
      <table style="border-collapse:collapse;width:100%;margin-bottom:18px">
        ${row("Yeni satış", `${s.count} adet &nbsp;<span style="color:#777;font-weight:400">(${s.monthly} aylık, ${s.yearly} yıllık)</span>`)}
        ${row("Gelir", `${money(s.revenue)} <span style="color:#777;font-weight:400">(liste fiyatı, USD)</span>`)}
        ${row("Satışta değişim", delta(s.count, m.prev.count))}
        ${row("Gelirde değişim", delta(s.revenue, m.prev.revenue))}
      </table>

      <h2 style="font-size:15px;margin:0 0 8px">Toplam durum</h2>
      <table style="border-collapse:collapse;width:100%;margin-bottom:18px">
        ${row("Aktif Pro lisans", `${m.activeTotal} <span style="color:#777;font-weight:400">(${m.activeMonthly} aylık, ${m.activeYearly} yıllık)</span>`)}
        ${row("Tahmini aylık gelir (MRR)", money(Math.round(m.mrr * 100) / 100))}
        ${row("Bu hafta kullanan kurulum", `${m.activeInstalls} <span style="color:#777;font-weight:400">(${m.freeInstalls} ücretsiz, ${m.newInstalls} yeni)</span>`)}
      </table>

      <div style="background:#f0f6ff;border:1px solid #cfe0fb;border-radius:8px;padding:14px;font-size:14px">
        <b>Reklam kârı:</b> Bu hafta reklamlara <b>${money(s.revenue)}</b>'dan az harcadıysan kârdasın.
        Harcama rakamı bu rapora otomatik girmiyor (Reddit/Google/Meta API'si gerekir) — onu sen ekle.
      </div>

      <p style="margin:18px 0 0;color:#999;font-size:12px">
        Gelir, ödenen yeni siparişlerden liste fiyatıyla hesaplanır; yenilemeler bu sayıya girmez.
      </p>
    </div>
  </div>
  </body></html>`;
}

/** Plain-text fallback, for mail clients that refuse the HTML. */
export function renderText(m: Metrics): string {
  const s = m.sales;
  return [
    `bubbleTranslate — haftalık özet`,
    ``,
    `Bu hafta:`,
    `  Yeni satış: ${s.count} (${s.monthly} aylık, ${s.yearly} yıllık) — ${delta(s.count, m.prev.count)}`,
    `  Gelir: ${money(s.revenue)} (liste, USD) — ${delta(s.revenue, m.prev.revenue)}`,
    ``,
    `Toplam:`,
    `  Aktif Pro lisans: ${m.activeTotal} (${m.activeMonthly} aylık, ${m.activeYearly} yıllık)`,
    `  Tahmini MRR: ${money(Math.round(m.mrr * 100) / 100)}`,
    `  Bu hafta kullanan kurulum: ${m.activeInstalls} (${m.freeInstalls} ücretsiz, ${m.newInstalls} yeni)`,
    ``,
    `Reklamlara ${money(s.revenue)}'dan az harcadıysan bu hafta kârdasın.`,
    `(Harcama otomatik girmiyor; onu sen ekle.)`,
  ].join("\n");
}

/** Where the report goes. A dedicated address if set, else support. */
const reportTo = (env: Env) => env.REPORT_EMAIL || env.SUPPORT_EMAIL || "support@bubbletranslate.app";

/** Builds and sends the weekly email. Best-effort: a mail failure is logged,
 *  never thrown, so a broken mailer cannot wedge the cron. */
export async function sendWeeklyReport(env: Env): Promise<void> {
  const m = await gather(env, Math.floor(Date.now() / 1000));
  if (!env.RESEND_API_KEY || !env.MAIL_FROM) {
    console.log("weekly report: no mailer configured; skipping send");
    console.log(renderText(m));
    return;
  }
  try {
    const res = await fetch("https://api.resend.com/emails", {
      method: "POST",
      headers: {
        authorization: `Bearer ${env.RESEND_API_KEY}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({
        from: env.MAIL_FROM,
        to: reportTo(env),
        subject: `bubbleTranslate haftalık: ${m.sales.count} satış, ${money(m.sales.revenue)}`,
        html: renderHtml(m),
        text: renderText(m),
      }),
    });
    if (!res.ok) throw new Error(`resend ${res.status}: ${await res.text()}`);
    console.log(`weekly report sent to ${reportTo(env)}`);
  } catch (err) {
    console.error("weekly report send failed", err);
  }
}

/** The same report as a page, for /admin/report -- so it can be seen now
 *  rather than waited for. */
export async function reportPage(env: Env): Promise<Response> {
  const m = await gather(env, Math.floor(Date.now() / 1000));
  return new Response(renderHtml(m), {
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}
