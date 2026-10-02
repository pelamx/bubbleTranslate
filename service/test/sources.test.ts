import { env } from "cloudflare:workers";
import { expect, it } from "vitest";
import worker, { cleanSource } from "../src/index";

const post = (path: string, body: unknown, country?: string, type = "application/json") => {
  const r = new Request(`https://api.bubbletranslate.app${path}`, {
    method: "POST",
    headers: { "content-type": type },
    body: JSON.stringify(body),
  });
  Object.defineProperty(r, "cf", { value: country ? { country } : undefined });
  return worker.fetch(r, env);
};

const install = (n: string) => n.repeat(32).slice(0, 32);
const sourceOf = async (id: string) =>
  (await env.DB.prepare("SELECT source FROM installs WHERE install = ?").bind(id).first<{ source: string | null }>())
    ?.source;

it("records a download click, sent as a beacon's plain text", async () => {
  const res = await post("/v1/download", { os: "linux", src: "Reddit" }, "DE", "text/plain;charset=UTF-8");
  expect(res.status).toBe(204);
  const row = await env.DB.prepare("SELECT os, src, country, install FROM download_clicks").first();
  expect(row).toEqual({ os: "linux", src: "reddit", country: "DE", install: null });
});

it("ignores a click for a system there is no build for", async () => {
  await post("/v1/download", { os: "android", src: "x" });
  const n = await env.DB.prepare("SELECT COUNT(*) AS n FROM download_clicks").first<{ n: number }>();
  expect(n?.n).toBe(0);
});

it("gives a new install the source of the matching click, and claims it", async () => {
  await post("/v1/download", { os: "linux", src: "hyprland" }, "DE");
  await post("/v1/download", { os: "windows", src: "reddit" }, "DE");
  await post("/v1/download", { os: "linux", src: "hn" }, "FR");

  const a = install("1");
  await post("/v1/ping", { install: a, os: "linux", app: "0.4.1" }, "DE");
  expect(await sourceOf(a)).toBe("hyprland");

  // The click is taken: a second Linux install from Germany finds nothing.
  const b = install("2");
  await post("/v1/ping", { install: b, os: "linux", app: "0.4.1" }, "DE");
  expect(await sourceOf(b)).toBe("unmatched");
});

it("never rewrites the source of an install it has already seen", async () => {
  const a = install("3");
  await env.DB.prepare(
    "INSERT INTO installs (install, os, app, plan, first_seen, last_seen) VALUES (?, 'linux', '0.4.0', 'free', 1, 1)",
  )
    .bind(a)
    .run();
  await post("/v1/download", { os: "linux", src: "reddit" }, "DE");
  await post("/v1/ping", { install: a, os: "linux", app: "0.4.1" }, "DE");
  expect(await sourceOf(a)).toBeNull();
  const claimed = await env.DB.prepare("SELECT install FROM download_clicks").first<{ install: string | null }>();
  expect(claimed?.install).toBeNull();
});

it("leaves a click older than the window unclaimed", async () => {
  await env.DB.prepare("INSERT INTO download_clicks (at, os, src, country) VALUES (?, 'macos', 'old', 'TR')")
    .bind(Math.floor(Date.now() / 1000) - 4 * 86_400)
    .run();
  const a = install("4");
  await post("/v1/ping", { install: a, os: "macos", app: "0.4.1" }, "TR");
  expect(await sourceOf(a)).toBe("unmatched");
});

it("keeps a source to a short plain word", () => {
  expect(cleanSource("www.Reddit.com")).toBe("reddit.com");
  expect(cleanSource("")).toBe("direct");
  expect(cleanSource("<script>")).toBe("other");
  expect(cleanSource("x".repeat(80))).toBe("other");
});
