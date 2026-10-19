import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";

const panel = { ...env, ADMIN_PASSWORD: "letmein", ANTHROPIC_ADMIN_KEY: "sk-ant-admin01-test" };
const auth = { authorization: `Basic ${btoa("op:letmein")}` };

const get = () => worker.fetch(new Request("https://api.bubbletranslate.app/admin", { headers: auth }), panel);
const post = (form: Record<string, string>) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/admin/anthropic-balance", {
      method: "POST",
      headers: auth,
      body: new URLSearchParams(form),
    }),
    panel,
  );

// The Cost API bills $1.25 a day for each day asked about; everything else
// the panel reads from outside is unavailable.
beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    const url = new URL(String(input));
    if (url.pathname.endsWith("/cost_report")) {
      const start = Date.parse(url.searchParams.get("starting_at")!);
      const data = [];
      for (let t = start; t < Date.now(); t += 86_400_000) {
        data.push({ starting_at: new Date(t).toISOString(), results: [{ amount: "125", description: "Claude Haiku" }] });
      }
      return Response.json({ data, has_more: false });
    }
    return new Response("", { status: 503 });
  });
});
afterEach(() => vi.restoreAllMocks());

it("asks for the Console's balance until one is entered", async () => {
  const html = await (await get()).text();
  expect(html).toContain("No balance entered yet");
  expect(html).toContain('action="/admin/anthropic-balance"');
});

it("takes the balance in any of the ways it is written, and counts it down", async () => {
  const res = await post({ dollars: "$20,00" });
  expect(res.status).toBe(303);
  expect(res.headers.get("location")).toBe("/admin#haiku");
  const row = await env.DB.prepare("SELECT dollars FROM anthropic_balance WHERE id = 1").first<{ dollars: number }>();
  expect(row?.dollars).toBe(20);
  // Entered today: today's $1.25 is counted against it.
  const html = (await (await get()).text()).replace(/<[^>]+>/g, "");
  expect(html).toContain("About $18.75 left");
});

it("refuses an amount it cannot read", async () => {
  expect((await post({ dollars: "lots" })).status).toBe(400);
  expect(await env.DB.prepare("SELECT * FROM anthropic_balance").first()).toBeNull();
});

it("shows the words the relay translated, with what they cost", async () => {
  const day = new Date().toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO word_usage (day, install, requests, input, output) VALUES (?, ?, ?, ?, ?)")
    .bind(day, "a".repeat(32), 120, 1_000_000, 200_000)
    .run();
  const html = (await (await get()).text()).replace(/<[^>]+>/g, "");
  expect(html).toContain("Today 120 of 40,000 words");
  // 1M in at $0.10 and 0.2M out at $0.50.
  expect(html).toContain("$0.200");
});
