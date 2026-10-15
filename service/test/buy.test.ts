import { env } from "cloudflare:workers";
import { expect, it } from "vitest";
import worker from "../src/index";
import { fromAcceptLanguage } from "../src/i18n";

const buy = (query: string, headers: Record<string, string> = {}) =>
  worker.fetch(new Request(`https://api.bubbletranslate.app/buy${query}`, { headers }), env as any);

it("reads the browser's language when nothing else says", () => {
  expect(fromAcceptLanguage("es-CL,es;q=0.9,en;q=0.8")).toBe("es");
  expect(fromAcceptLanguage("tr-TR")).toBe("tr");
  expect(fromAcceptLanguage("de-DE,tr;q=0.5,en;q=0.7")).toBe("en");
  expect(fromAcceptLanguage("es;q=0,tr")).toBe("tr");
  expect(fromAcceptLanguage("de,fr")).toBeUndefined();
  expect(fromAcceptLanguage(null)).toBeUndefined();
});

it("opens in the browser's language, and ?lang= still wins", async () => {
  const spanish = await (await buy("?src=bubble", { "accept-language": "es-CL,es;q=0.9" })).text();
  expect(spanish).toContain('<html lang="es"');
  const asked = await (await buy("?src=bubble&lang=tr", { "accept-language": "es-CL" })).text();
  expect(asked).toContain('<html lang="tr"');
});

it("counts each visit by where it came from", async () => {
  await buy("?src=bubble&lang=es");
  await buy("");
  await buy("?src=<script>");
  const { results } = await env.DB.prepare("SELECT src, lang FROM buy_visits ORDER BY id").all();
  expect(results).toEqual([
    { src: "bubble", lang: "es" },
    { src: "direct", lang: "en" },
    { src: "other", lang: "en" },
  ]);
});

it("keeps the app's own sources, hyphen and all, and stamps the order with it", async () => {
  await buy("?src=bubble-low");
  const visit = await env.DB.prepare("SELECT src FROM buy_visits ORDER BY id DESC").first();
  expect(visit).toEqual({ src: "bubble-low" });

  const res = await worker.fetch(
    new Request("https://api.bubbletranslate.app/checkout/paddle", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ cycle: "yearly", email: "a@example.com", src: "bubble-low" }),
    }),
    env as any,
  );
  const { ref } = (await res.json()) as { ref: string };
  const order = await env.DB.prepare("SELECT src FROM orders WHERE ref = ?").bind(ref).first();
  expect(order).toEqual({ src: "bubble-low" });
});

it("asks why on the cancel form, in the page's language", async () => {
  const { accountPage } = await import("../src/pages");
  const ctx = { lang: "tr", url: new URL("https://api.bubbletranslate.app/account") } as any;
  const html = await accountPage(ctx, {
    key: "BT-AAAAA-BBBBB-CCCCC", cancellable: true, renews: "2027-01-01",
  } as any, "x@example.com").text();
  expect(html).toContain('name="reason"');
  expect(html).toContain("Fiyatı yüksek");
  expect(html).toContain('name="note"');
});
