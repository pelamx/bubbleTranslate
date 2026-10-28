import { env } from "cloudflare:workers";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import worker from "../src/index";
const adminEnv = { ...env, ADMIN_PASSWORD: "letmein" };
const auth = { authorization: `Basic ${btoa("op:letmein")}` };
const base = "https://api.bubbletranslate.app";
const day = Math.floor(Date.now() / 86400000);
const metric = (count: number, extra = {}) => ({ day, app: "0.4.20", feature: "selection", outcome: "ok", count, ...extra });
const ping = (metrics: unknown) => worker.fetch(new Request(`${base}/v1/ping`, {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ install: "d".repeat(32), os: "linux", app: "0.4.20", channel: "download", metrics }),
}), env);
const save = (settings: unknown, os = "*", channel = "*", origin = base) => worker.fetch(new Request(`${base}/admin/remote-config`, {
  method: "POST", headers: { ...auth, origin },
  body: new URLSearchParams({ os, channel, settings: JSON.stringify(settings) }),
}), adminEnv);
const config = async (os = "linux", channel = "download") => (await worker.fetch(new Request(`${base}/v1/config?os=${os}&channel=${channel}`), env)).json();
const panel = async () => (await worker.fetch(new Request(`${base}/admin`, {headers: auth}), adminEnv)).text();
beforeEach(() => { vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", {status: 503})); });
afterEach(() => vi.restoreAllMocks());

it("does not double-count duplicate or reordered snapshots, and keeps versions separate", async () => {
  for (const n of [3, 3, 2, 5]) expect((await ping([metric(n)])).status).toBe(204);
  await ping([metric(2, { app: "0.4.21" })]);
  expect((await env.DB.prepare("SELECT app, count FROM feature_metrics ORDER BY app").all()).results)
    .toEqual([{app: "0.4.20", count: 5}, {app: "0.4.21", count: 2}]);
});
it("drops unknown names, invalid counts and dates", async () => {
  await ping([metric(1, {feature: "secret text"}), metric(1, {outcome: "raw error"}), metric(-1), metric(1.5), metric(1, {day: day - 40}), metric(1, {day: day + 2}), metric(1, {app: "<script>"}), metric(1, {feature: "ocr_box", outcome: "translation_failed"})]);
  expect((await env.DB.prepare("SELECT feature, outcome, count FROM feature_metrics").all()).results)
    .toEqual([{feature: "ocr_box", outcome: "translation_failed", count: 1}]);
});
it("shows successful users, repeat use and errors by stage", async () => {
  await ping([metric(3), metric(2, {day: day - 1}), metric(1, {feature: "app", outcome: "panic"})]);
  const html = await panel();
  expect(html).toContain("Feature analytics, last 30 days");
  expect(html).toContain("1 of 1 reporting installs completed a translation");
  expect(html).toContain("app / panic");
  expect(html).toContain("<td>selection</td><td>1</td><td>1</td>");
});
it("merges scopes, clears overrides, and audits changes", async () => {
  expect(await config()).toEqual({});
  expect((await save({debounce_ms: 250, dictionary_enabled: false})).status).toBe(303);
  await save({debounce_ms: 400}, "linux");
  await save({debounce_ms: 500}, "*", "store");
  await save({debounce_ms: 700}, "linux", "store");
  expect(await config()).toEqual({debounce_ms: 400, dictionary_enabled: false});
  expect(await config("windows")).toEqual({debounce_ms: 250, dictionary_enabled: false});
  expect(await config("linux", "store")).toEqual({debounce_ms: 700, dictionary_enabled: false});
  await save({}, "linux", "store");
  expect(await config("linux", "store")).toEqual({debounce_ms: 500, dictionary_enabled: false});
  expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM admin_log WHERE action='remote-config'").first<any>())?.n).toBe(5);
});
it("requires authentication, rejects cross-site changes and validates remote settings", async () => {
  expect((await worker.fetch(new Request(`${base}/admin/remote-config`, {method: "POST"}), adminEnv)).status).toBe(401);
  expect((await save({}, "*", "*", "https://attacker.example")).status).toBe(403);
  for (const s of [{debounce_ms: -1}, {debounce_ms: 2001}, {debounce_ms: 1.5}, {dictionary_enabled: "false"}, {endpoint: "https://attacker.example"}, {update_message: "x".repeat(301)}, []]) expect((await save(s)).status).toBe(400);
  expect((await save({translation_timeout_ms: 1500})).status).toBe(400);
  expect((await save({disabled_providers: ["unknown"]})).status).toBe(400);
  expect((await save({update_before_version: "bad"})).status).toBe(400);
  expect((await save({}, "android")).status).toBe(400);
  expect(await config()).toEqual({});
});
it("escapes remote messages in the admin panel", async () => {
  await save({update_message: "<script>alert(1)</script>"});
  const html = await panel();
  expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
  expect(html).not.toContain("<script>alert(1)</script>");
});

it("accepts bounded provider controls and version-scoped update messages", async () => {
  const settings = {translation_timeout_ms: 12000, disabled_providers: ["google", "claude"], update_message: "Update", update_before_version: "0.4.22"};
  expect((await save(settings, "linux", "download")).status).toBe(303);
  expect(await config()).toEqual(settings);
  expect(await config("windows")).toEqual({});
});

it("sets the free allowance only for copies that say their version, and says so on the buy page", async () => {
  const before = await (await worker.fetch(new Request(`${base}/buy?lang=en`), env)).text();
  expect(before).toContain("3 text translations, 3 screen boxes and 3 whole screens a day");
  for (const n of [2, 11, 4.5, "5"]) expect((await save({free_daily_translations: n})).status).toBe(400);
  expect((await save({free_daily_translations: 5, debounce_ms: 300})).status).toBe(303);
  // A copy before 0.4.23 would reject the whole answer over a control it does
  // not know, so it is not sent one.
  expect(await config()).toEqual({debounce_ms: 300});
  const versioned = await (await worker.fetch(new Request(`${base}/v1/config?os=linux&channel=download&app=0.4.23`), env)).json();
  expect(versioned).toEqual({debounce_ms: 300, free_daily_translations: 5});
  const junk = await (await worker.fetch(new Request(`${base}/v1/config?os=linux&channel=download&app=<x>`), env)).json();
  expect(junk).toEqual({debounce_ms: 300});
  const buy = await (await worker.fetch(new Request(`${base}/buy?lang=en`), env)).text();
  expect(buy).toContain("5 text translations, 3 screen boxes and 3 whole screens a day");
  expect(buy).toContain("allows 5 translations of selected text");
});
