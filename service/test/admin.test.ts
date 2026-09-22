import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { issueLicence } from "../src/licences";

const withPassword = { ...env, ADMIN_PASSWORD: "letmein" };
const get = (path: string, e: any = withPassword, auth = "letmein") =>
  worker.fetch(
    new Request(`https://api.bubbletranslate.app${path}`, {
      headers: auth ? { authorization: `Basic ${btoa(`op:${auth}`)}` } : {},
    }),
    e,
  );

// The download counts come from GitHub; the panel must render without them.
beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", { status: 503 }));
});
afterEach(() => vi.restoreAllMocks());

it("does not exist when no password is configured", async () => {
  expect((await get("/admin", { ...env, ADMIN_PASSWORD: undefined })).status).toBe(404);
});

it("asks for the password", async () => {
  expect((await get("/admin", withPassword, "")).status).toBe(401);
});

it("renders the dashboard and finds a licence by email", async () => {
  await issueLicence(env, { provider: "paddle", cycle: "yearly", email: "someone@example.com" });
  const res = await get("/admin?q=someone%40example.com");
  expect(res.status).toBe(200);
  const html = await res.text();
  expect(html).toContain("someone@example.com");
  expect(html).toContain("yearly");
});
