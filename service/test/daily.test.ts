import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker, { forgetOld } from "../src/index";
import { findAlerts, sendAlerts } from "../src/alerts";
import { churn } from "../src/admin/growth";
import { issueLicence } from "../src/licences";

const withPassword = { ...env, ADMIN_PASSWORD: "letmein" };
const admin = () =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/admin", {
      headers: { authorization: `Basic ${btoa("op:letmein")}` },
    }),
    withPassword,
  );
const ping = (install: string, extra: Record<string, unknown>) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/ping", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ install, os: "windows", app: "0.4.19", plan: "free", ...extra }),
    }),
    env,
  );
const today = () => Math.floor(Date.now() / 86_400_000);
const iso = (day: number) => new Date(day * 86_400_000).toISOString().slice(0, 10);

beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", { status: 503 }));
});
afterEach(() => vi.restoreAllMocks());

it("files backend tallies under the day they happened, not the day they arrived", async () => {
  const yesterday = today() - 1;
  await ping("1".repeat(32), {
    daily: [
      { day: yesterday, section: "providers", name: "Google", counts: { ok: 40, failed: 2 } },
      { day: yesterday, section: "providers", name: "MyMemory", counts: { ok: 3 } },
      { day: yesterday, section: "nonsense", name: "x", counts: { ok: 1 } },
    ],
  });
  const { results } = await env.DB.prepare("SELECT * FROM provider_health ORDER BY provider").all<any>();
  expect(results).toEqual([
    { day: iso(yesterday), provider: "Google", ok: 40, failed: 2, reports: 1 },
    { day: iso(yesterday), provider: "MyMemory", ok: 3, failed: 0, reports: 1 },
  ]);
});

it("counts screen readings, and shows them", async () => {
  const day = today();
  await ping("2".repeat(32), {
    daily: [
      { day, section: "screen", name: "page", counts: { done: 3, capped: 2 } },
      { day, section: "screen", name: "box", counts: { done: 1 } },
      { day, section: "screen", name: "elsewhere", counts: { done: 9 } },
    ],
  });
  const { results } = await env.DB.prepare("SELECT kind, done, capped FROM screen_usage ORDER BY kind").all<any>();
  expect(results).toEqual([
    { kind: "box", done: 1, capped: 0 },
    { kind: "page", done: 3, capped: 2 },
  ]);
  const html = await (await admin()).text();
  const card = html.slice(html.indexOf('id="screen"'));
  expect(card).toContain("Screen readings, last 30 days");
  expect(card).toContain("<b>2</b> <span class=\"muted\">by 1</span>");
});

it("keeps a store install apart from website downloads", async () => {
  // A website click that a website install would be matched to.
  await worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/download", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ os: "windows", src: "google" }),
    }),
    env,
  );
  await ping("3".repeat(32), { channel: "store" });
  await ping("4".repeat(32), { channel: "download" });
  const rows = (
    await env.DB.prepare("SELECT install, channel, source FROM installs ORDER BY install").all<any>()
  ).results;
  expect(rows).toEqual([
    { install: "3".repeat(32), channel: "store", source: "store" },
    { install: "4".repeat(32), channel: "download", source: "google" },
  ]);
  const html = await (await admin()).text();
  expect(html).toContain("Website or store");
});

it("shows the DeepL fallback against its daily cap", async () => {
  const day = new Date().toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO deepl_usage (day, install, requests, chars) VALUES (?, ?, 51, 7500)")
    .bind(day, "5".repeat(32))
    .run();
  const html = await (await admin()).text();
  const card = html.slice(html.indexOf('id="deepl"'));
  expect(card).toContain("Today 7,500 of 15,000 characters");
  expect(card).toContain("capped");
});

it("counts a licence that ran out in the month as churn, and a renewed one not", async () => {
  const d = new Date();
  const lastMonth = Date.UTC(d.getUTCFullYear(), d.getUTCMonth() - 1, 1) / 1000;
  const thisMonth = Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1) / 1000;
  const a = await issueLicence(env, { provider: "paddle", cycle: "monthly", email: "a@example.com" });
  const b = await issueLicence(env, { provider: "paddle", cycle: "monthly", email: "b@example.com" });
  await env.DB.batch([
    env.DB.prepare("UPDATE licences SET created_at = ?, expires_at = ? WHERE id = ?").bind(
      lastMonth - 86_400,
      lastMonth + 10 * 86_400,
      a.id,
    ),
    env.DB.prepare("UPDATE licences SET created_at = ?, expires_at = ? WHERE id = ?").bind(
      lastMonth - 86_400,
      thisMonth + 20 * 86_400,
      b.id,
    ),
  ]);
  const months = await churn(env);
  const label = new Date(lastMonth * 1000).toISOString().slice(0, 7);
  expect(months.find((m) => m.month === label)).toEqual({ month: label, start: 2, ended: 1 });
});

it("raises an alert when the fallback answers too much, and not when it is quiet", async () => {
  const t = Math.floor(Date.now() / 1000);
  const yesterday = new Date((t - 86_400) * 1000).toISOString().slice(0, 10);
  expect(await findAlerts(env, t)).toEqual([]);
  await env.DB.batch([
    env.DB.prepare("INSERT INTO provider_health (day, provider, ok, failed, reports) VALUES (?, 'Google', 60, 0, 5)").bind(yesterday),
    env.DB.prepare("INSERT INTO provider_health (day, provider, ok, failed, reports) VALUES (?, 'MyMemory', 20, 0, 5)").bind(yesterday),
  ]);
  const alerts = await findAlerts(env, t);
  expect(alerts.map((a) => a.kind)).toEqual(["fallback"]);
  expect(alerts[0].text).toContain("%25");
});

it("mails an alert once a day, however often the cron runs", async () => {
  const t = Math.floor(Date.now() / 1000);
  const yesterday = new Date((t - 86_400) * 1000).toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO provider_health (day, provider, ok, failed, reports) VALUES (?, 'Google', 0, 30, 5)")
    .bind(yesterday)
    .run();
  const sent = vi.mocked(globalThis.fetch).mockResolvedValue(new Response("{}", { status: 200 }));
  const e = { ...env, RESEND_API_KEY: "re_test", MAIL_FROM: "a@b.c" } as any;
  await sendAlerts(e);
  await sendAlerts(e);
  const mails = sent.mock.calls.filter(([url]) => String(url).includes("resend"));
  expect(mails).toHaveLength(1);
  expect(String(mails[0][1]?.body)).toContain("30 seçim");
});

it("counts translations by plan, and how much of the free ten they used", async () => {
  const day = today();
  await ping("3".repeat(32), {
    daily: [
      { day, section: "providers", name: "Google", counts: { ok: 14 } },
      { day, section: "translate", name: "free", counts: { done: 12, charged: 10, capped: 3 } },
      { day, section: "translate", name: "elsewhere", counts: { done: 9 } },
    ],
  });
  await ping("4".repeat(32), {
    daily: [{ day, section: "translate", name: "free", counts: { done: 2, charged: 2 } }],
  });
  const { results } = await env.DB.prepare(
    "SELECT install, plan, done, charged, capped FROM translate_usage ORDER BY install",
  ).all<any>();
  expect(results).toEqual([
    { install: "3".repeat(32), plan: "free", done: 12, charged: 10, capped: 3 },
    { install: "4".repeat(32), plan: "free", done: 2, charged: 2, capped: 0 },
  ]);
  const html = await (await admin()).text();
  const card = html.slice(html.indexOf('id="usage"'));
  expect(card).toContain("Translations a day, last 30 days");
  expect(card).toContain("<b>14</b>");
  expect(card).toContain("14 <span class=\"muted\">by 2</span>");
  expect(card).toContain("<b>50%</b> <span class=\"muted\">1</span>");
});

it("splits the translations by the version that reported them", async () => {
  const day = today();
  const old = (install: string, extra: Record<string, unknown>) =>
    worker.fetch(
      new Request("https://api.bubbletranslate.app/v1/ping", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ install, os: "windows", plan: "free", ...extra }),
      }),
      env,
    );
  // Two versions on the same day, and a copy that reports no version at all.
  await ping("a".repeat(32), {
    app: "0.4.23",
    daily: [{ day, section: "translate", name: "free", counts: { done: 10, charged: 10, capped: 2 } }],
  });
  await ping("b".repeat(32), {
    app: "0.4.22",
    daily: [{ day, section: "translate", name: "free", counts: { done: 3, charged: 3 } }],
  });
  await old("c".repeat(32), {
    daily: [{ day, section: "translate", name: "free", counts: { done: 1, charged: 1 } }],
  });
  const { results } = await env.DB.prepare(
    "SELECT install, app, done FROM translate_usage ORDER BY install",
  ).all<any>();
  expect(results).toEqual([
    { install: "a".repeat(32), app: "0.4.23", done: 10 },
    { install: "b".repeat(32), app: "0.4.22", done: 3 },
    { install: "c".repeat(32), app: null, done: 1 },
  ]);

  const html = await (await admin()).text();
  const from = html.slice(html.indexOf('id="usage"'));
  const card = from.slice(0, from.indexOf("</section>"));
  expect(card).toContain("Per version");
  // Busiest first, and a version with no name grouped rather than dropped.
  expect(card.indexOf("0.4.23")).toBeLessThan(card.indexOf("0.4.22"));
  expect(card).toContain("not recorded");
  // One free day of the ten reached, out of 0.4.23's one.
  expect(card).toContain('<b>100%</b> <span class="muted">1/1</span>');
});

it("lists each free install day by day, the one that ran out first", async () => {
  const day = today();
  const close = "6".repeat(32);
  const light = "7".repeat(32);
  await ping(close, {
    daily: [
      { day: day - 1, section: "translate", name: "free", counts: { done: 5, charged: 3, capped: 4 } },
      { day, section: "translate", name: "free", counts: { done: 2, charged: 2 } },
      { day, section: "translate", name: "pro", counts: { done: 30 } },
    ],
  });
  await ping(light, { daily: [{ day, section: "translate", name: "free", counts: { done: 1, charged: 1 } }] });
  const html = await (await admin()).text();
  const from = html.slice(html.indexOf('id="installs-use"'));
  const card = from.slice(0, from.indexOf("</section>"));
  expect(card).toContain("Free translations per install, day by day");
  expect(card.indexOf(close.slice(0, 8))).toBeLessThan(card.indexOf(light.slice(0, 8)));
  expect(card).toContain('<td class="d full">3<small>+4 refused</small></td>');
  expect(card).toContain('<td class="d high">2</td>');
  expect(card).toContain('<td class="d">1</td>');
  expect(card).toContain("Days at 3");
});

it("forgets silent installs and stale unpaid orders once a day, and keeps the rest", async () => {
  const t = Math.floor(Date.now() / 1000);
  await env.DB.batch([
    env.DB.prepare("INSERT INTO installs (install, first_seen, last_seen) VALUES ('old', ?1, ?1), ('new', ?2, ?2)")
      .bind(t - 400 * 86_400, t),
    env.DB.prepare(
      `INSERT INTO orders (ref, provider, cycle, status, created_at) VALUES
         ('stale', 'paddle', 'monthly', 'pending', ?1),
         ('sold', 'paddle', 'monthly', 'paid', ?1),
         ('fresh', 'paddle', 'monthly', 'pending', ?2)`,
    ).bind(t - 100 * 86_400, t),
  ]);
  await forgetOld(env);
  const installs = await env.DB.prepare("SELECT install FROM installs ORDER BY install").all<any>();
  expect(installs.results.map((r) => r.install)).toEqual(["new"]);
  const orders = await env.DB.prepare("SELECT ref FROM orders ORDER BY ref").all<any>();
  expect(orders.results.map((r) => r.ref)).toEqual(["fresh", "sold"]);
});
