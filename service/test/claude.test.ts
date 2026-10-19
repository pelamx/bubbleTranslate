import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { anthropicAccount, listCost, summariseAccount } from "../src/admin/claude";

const withPassword = { ...env, ADMIN_PASSWORD: "letmein" };
const admin = (e: any = withPassword) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/admin", {
      headers: { authorization: `Basic ${btoa("op:letmein")}` },
    }),
    e,
  );
/** The client's daily entries, from the old flat Claude rows the tests are
 *  written in: anything that is not a row passes through as junk. */
const ping = (install: string, claude: any[]) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/ping", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        install,
        os: "linux",
        app: "0.4.19",
        plan: "free",
        daily: claude.map((r) =>
          r && typeof r === "object"
            ? { day: r.day, section: "claude", name: r.model, counts: { ...r, day: undefined, model: undefined } }
            : r,
        ),
      }),
    }),
    env,
  );
const today = () => Math.floor(Date.now() / 86_400_000);

beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", { status: 503 }));
});
afterEach(() => vi.restoreAllMocks());

it("prices Haiku at list price, and leaves an unknown model unpriced", () => {
  // 1M in at $0.10 and 200K out at $0.50.
  expect(listCost("claude-haiku-5-5", { input: 1e6, output: 2e5, cache_read: 0, cache_write: 0 })).toBeCloseTo(0.2);
  expect(listCost("claude-unknown", { input: 1e6, output: 0, cache_read: 0, cache_write: 0 })).toBeNull();
});

it("adds up what a ping reports, and drops what it should not believe", async () => {
  const install = "c".repeat(32);
  const day = today();
  const row = { day, model: "claude-haiku-5-5", requests: 3, failed: 1, input: 6000, output: 900 };
  expect((await ping(install, [row])).status).toBe(204);
  // The same day again: a resend after a lost answer adds to it.
  await ping(install, [
    { ...row, requests: 1, failed: 0, input: 2000, output: 300 },
    { ...row, day: day - 400 },
    { ...row, model: "<script>" },
    "junk",
  ]);
  const { results } = await env.DB.prepare("SELECT * FROM claude_usage").all<any>();
  expect(results).toHaveLength(1);
  expect(results[0]).toMatchObject({
    day: new Date(day * 86_400_000).toISOString().slice(0, 10),
    install,
    os: "linux",
    requests: 4,
    failed: 1,
    input: 8000,
    output: 1200,
  });
});

it("shows what users' keys cost, without the operator's own machines", async () => {
  const theirs = "d".repeat(32);
  const mine = "e".repeat(32);
  const day = today();
  await ping(theirs, [{ day, model: "claude-haiku-5-5", requests: 10, input: 1_000_000, output: 200_000 }]);
  await ping(mine, [{ day, model: "claude-haiku-5-5", requests: 99, input: 9_000_000, output: 900_000 }]);
  const html = await (await admin({ ...withPassword, ADMIN_IGNORE_INSTALLS: mine })).text();
  const card = html.slice(html.indexOf('id="claude"'));
  expect(card).toContain("haiku-5-5");
  // $0.10 + $0.10, and 99 requests from the operator's machine are not in it.
  expect(card).toContain("$0.200");
  expect(card).not.toContain(">99<");
  expect(card).toContain("No Anthropic Admin API key is configured.");
});

it("reads the organisation's bill from Anthropic", async () => {
  const day = new Date().toISOString().slice(0, 10);
  const spy = vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = new URL(String(input));
    expect(new Headers(init?.headers).get("x-api-key")).toBe("sk-ant-admin01-test");
    if (url.pathname.endsWith("/cost_report")) {
      expect(url.searchParams.getAll("group_by[]")).toEqual(["description"]);
      return Response.json({
        data: [
          {
            starting_at: `${day}T00:00:00Z`,
            ending_at: `${day}T00:00:00Z`,
            results: [
              { amount: "12.5", currency: "USD", model: "claude-haiku-5-5", description: "Claude Haiku 5.5 Usage" },
              { amount: "30", currency: "USD", model: null, description: "Web Search Usage" },
            ],
          },
        ],
        has_more: false,
        next_page: null,
      });
    }
    return Response.json({
      data: [
        {
          starting_at: `${day}T00:00:00Z`,
          results: [
            {
              model: "claude-haiku-5-5",
              uncached_input_tokens: 1_000_000,
              output_tokens: 50_000,
              cache_read_input_tokens: 0,
              cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_1h_input_tokens: 0 },
            },
          ],
        },
      ],
      has_more: false,
      next_page: null,
    });
  });
  const a = await anthropicAccount({ ...env, ANTHROPIC_ADMIN_KEY: "sk-ant-admin01-test" } as any);
  expect(spy).toHaveBeenCalledTimes(2);
  if (typeof a === "string") throw new Error(a);
  // Cents in, dollars out.
  expect(a.total).toBeCloseTo(0.425);
  expect(a.today).toBeCloseTo(0.425);
  expect(a.models[0]).toMatchObject({ model: "Web Search Usage", cost: 0.3 });
  expect(a.models[1]).toMatchObject({ model: "claude-haiku-5-5", input: 1_000_000, output: 50_000 });
});

it("passes on Anthropic's reason when it refuses the key", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    Response.json({ type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } }, { status: 401 }),
  );
  expect(await anthropicAccount({ ...env, ANTHROPIC_ADMIN_KEY: "sk-ant-api03-wrong" } as any)).toBe(
    "Anthropic answered 401: invalid x-api-key",
  );
});

it("keeps an empty day in the series", () => {
  const a = summariseAccount([{ starting_at: "2026-10-01T00:00:00Z", results: [] }], []);
  expect(a.days).toEqual([{ day: "2026-10-01", cost: 0 }]);
  expect(a.models).toEqual([]);
});
