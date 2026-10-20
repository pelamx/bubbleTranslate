import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { WORD_DAILY, WORD_PER_INSTALL } from "../src/word";

const install = (n: string) => n.repeat(32).slice(0, 32);

const ask = (body: unknown, e: any = { ...env, ANTHROPIC_API_KEY: "sk-ant-test" }) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/word", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    e,
  );

const bank = { install: install("a"), word: "bank", sentence: "We sat on the bank of the river.", target: "tr", alt: "en" };

let calls: { url: string; init: RequestInit }[];
let reply: () => Response;

beforeEach(() => {
  calls = [];
  reply = () =>
    Response.json({
      stop_reason: "end_turn",
      content: [{ type: "text", text: JSON.stringify({ source: "EN", text: "kıyı" }) }],
      usage: { input_tokens: 180, output_tokens: 12 },
    });
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    calls.push({ url: String(input), init: init ?? {} });
    return reply();
  });
});

afterEach(() => vi.restoreAllMocks());

it("asks Claude Haiku with the secret key and hands back the translation", async () => {
  const res = await ask(bank);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ text: "kıyı", source: "en" });
  expect(calls).toHaveLength(1);
  expect(calls[0].url).toBe("https://api.anthropic.com/v1/messages");
  expect((calls[0].init.headers as any)["x-api-key"]).toBe("sk-ant-test");
  const sent = JSON.parse(calls[0].init.body as string);
  expect(sent.model).toBe("claude-haiku-5-5");
  expect(sent.system).toContain("into Turkish, or into English if the word is itself in Turkish");
  expect(JSON.parse(sent.messages[0].content)).toEqual({ word: "bank", sentence: bank.sentence });
});

it("answers 503 without asking Anthropic when no key is configured", async () => {
  expect((await ask(bank, env)).status).toBe(503);
  expect(calls).toHaveLength(0);
});

it("refuses malformed requests and overlong text", async () => {
  for (const body of [
    { ...bank, install: "nope" },
    { ...bank, word: "" },
    { ...bank, sentence: "  " },
    { ...bank, target: "tr&x=1" },
    { ...bank, alt: "../" },
  ]) {
    expect((await ask(body)).status).toBe(400);
  }
  expect((await ask({ ...bank, sentence: "x".repeat(5000) })).status).toBe(413);
  expect(calls).toHaveLength(0);
});

it("stops one install at its daily allowance, and every install at the day's", async () => {
  const day = new Date().toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO word_usage (day, install, requests) VALUES (?, ?, ?)")
    .bind(day, install("a"), WORD_PER_INSTALL)
    .run();
  expect((await ask(bank)).status).toBe(429);
  // Another install still gets an answer, until the day's total is spent.
  expect((await ask({ ...bank, install: install("b") })).status).toBe(200);
  await env.DB.prepare("INSERT INTO word_usage (day, install, requests) VALUES (?, ?, ?)")
    .bind(day, install("c"), WORD_DAILY)
    .run();
  expect((await ask({ ...bank, install: install("d") })).status).toBe(429);
  expect(calls).toHaveLength(1);
});

it("passes Anthropic's busy signal on as a rate limit, and anything else as a failure", async () => {
  reply = () => new Response("{}", { status: 529 });
  expect((await ask(bank)).status).toBe(429);
  reply = () => new Response("{}", { status: 500 });
  expect((await ask(bank)).status).toBe(502);
  reply = () => Response.json({ stop_reason: "refusal", content: [] });
  expect((await ask(bank)).status).toBe(502);
});

it("counts the tokens each install's words took, never the words", async () => {
  await ask(bank);
  await ask(bank);
  const row = await env.DB.prepare("SELECT * FROM word_usage").first<any>();
  expect(row).toMatchObject({ install: install("a"), requests: 2, input: 360, output: 24 });
  expect(JSON.stringify(row)).not.toContain("bank");
});

it("never spends the shared allowance on a refused install", async () => {
  const day = new Date().toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO word_usage (day, install, requests) VALUES (?, ?, ?)")
    .bind(day, bank.install, WORD_PER_INSTALL).run();
  for (let i = 0; i < 3; i++) expect((await ask(bank)).status).toBe(429);
  const row = await env.DB.prepare("SELECT SUM(requests) AS requests FROM word_usage WHERE day = ?").bind(day).first<any>();
  expect(row.requests).toBe(WORD_PER_INSTALL);
  expect(calls).toHaveLength(0);
});

it("reserves the last daily slot only once under concurrent requests", async () => {
  const day = new Date().toISOString().slice(0, 10);
  await env.DB.prepare("INSERT INTO word_usage (day, install, requests) VALUES (?, ?, ?)")
    .bind(day, install("f"), WORD_DAILY - 1).run();
  const responses = await Promise.all(["a", "b", "c", "d"].map(n => ask({ ...bank, install: install(n) })));
  expect(responses.filter(r => r.status === 200)).toHaveLength(1);
  expect(responses.filter(r => r.status === 429)).toHaveLength(3);
  expect(calls).toHaveLength(1);
});

it("caps response cost and refuses a truncated translation while counting its tokens", async () => {
  reply = () => Response.json({ stop_reason: "max_tokens", content: [{ type: "text", text: JSON.stringify({ text: "partial", source: "en" }) }], usage: { input_tokens: 180, output_tokens: 256 } });
  expect((await ask(bank)).status).toBe(502);
  const sent = JSON.parse(calls[0].init.body as string);
  expect(sent.max_tokens).toBe(256);
  expect(sent.output_config.format.schema.properties.text.maxLength).toBe(200);
  const row = await env.DB.prepare("SELECT output FROM word_usage").first<any>();
  expect(row.output).toBe(256);
});