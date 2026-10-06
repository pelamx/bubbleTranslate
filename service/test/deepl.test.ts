import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { DEEPL_DAILY_CHARS, DEEPL_PER_INSTALL } from "../src/deepl";

const install = (n: string) => n.repeat(32).slice(0, 32);

const ask = (body: unknown, e: any = { ...env, DEEPL_API_KEY: "test-key:fx" }) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/deepl", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    e,
  );

let calls: { url: string; init: RequestInit }[];

beforeEach(() => {
  calls = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    calls.push({ url: String(input), init: init ?? {} });
    return Response.json({ translations: [{ text: "Merhaba dünya", detected_source_language: "EN" }] });
  });
});

afterEach(() => vi.restoreAllMocks());

it("forwards the text to DeepL with the secret key and hands back its answer", async () => {
  const res = await ask({ install: install("a"), text: ["Hello world"], target_lang: "TR" });
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({
    translations: [{ text: "Merhaba dünya", detected_source_language: "EN" }],
  });
  expect(calls).toHaveLength(1);
  // A free key goes to the free host.
  expect(calls[0].url).toBe("https://api-free.deepl.com/v2/translate");
  expect((calls[0].init.headers as any).authorization).toBe("DeepL-Auth-Key test-key:fx");
  expect(JSON.parse(calls[0].init.body as string)).toEqual({ text: ["Hello world"], target_lang: "TR" });
});

it("answers 503 without asking DeepL when no key is configured", async () => {
  const res = await ask({ install: install("a"), text: ["Hello"], target_lang: "TR" }, env);
  expect(res.status).toBe(503);
  expect(calls).toHaveLength(0);
});

it("refuses malformed requests and overlong text", async () => {
  for (const body of [
    { install: "nope", text: ["Hello"], target_lang: "TR" },
    { install: install("a"), text: [""], target_lang: "TR" },
    { install: install("a"), text: ["Hello"], target_lang: "tr&x=1" },
    { install: install("a"), text: ["Hello"], target_lang: "TR", source_lang: "../" },
  ]) {
    expect((await ask(body)).status).toBe(400);
  }
  expect((await ask({ install: install("a"), text: ["x".repeat(5000)], target_lang: "TR" })).status).toBe(413);
  expect(calls).toHaveLength(0);
});

it("stops one install after its daily share", async () => {
  for (let i = 0; i < DEEPL_PER_INSTALL; i++) {
    expect((await ask({ install: install("a"), text: ["Hi"], target_lang: "TR" })).status).toBe(200);
  }
  expect((await ask({ install: install("a"), text: ["Hi"], target_lang: "TR" })).status).toBe(429);
  // Another copy is not held to the first one's count.
  expect((await ask({ install: install("b"), text: ["Hi"], target_lang: "TR" })).status).toBe(200);
});

it("stops everyone once the day's characters are spent", async () => {
  await env.DB.prepare("INSERT INTO deepl_usage (day, install, requests, chars) VALUES (?, ?, 1, ?)")
    .bind(new Date().toISOString().slice(0, 10), install("z"), DEEPL_DAILY_CHARS - 3)
    .run();
  const res = await ask({ install: install("a"), text: ["Hello"], target_lang: "TR" });
  expect(res.status).toBe(429);
  expect(calls).toHaveLength(0);
});

it("turns DeepL's spent-quota answer into a 429", async () => {
  vi.mocked(globalThis.fetch).mockResolvedValue(new Response("", { status: 456 }));
  const res = await ask({ install: install("a"), text: ["Hello"], target_lang: "TR" });
  expect(res.status).toBe(429);
});
