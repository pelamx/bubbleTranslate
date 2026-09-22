import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import worker from "../src/index";
import {
  GRACE_SECONDS,
  type Licence,
  TOKEN_TTL,
  endLicence,
  grantToken,
  isLive,
  issueLicence,
  licenceById,
  newLicenceKey,
  refusalFor,
} from "../src/licences";
import { now, open } from "../src/tokens";

const base = (over: Partial<Licence> = {}): Licence => ({
  id: "lc_test",
  plan: "pro",
  cycle: "monthly",
  translation_limit: null,
  status: "active",
  seat_limit: 3,
  expires_at: now() + 86_400,
  renews_at: null,
  email: null,
  provider: "paddle",
  provider_ref: "sub_1",
  ...over,
});

const post = (path: string, body: object) =>
  worker.fetch(
    new Request(`https://api.bubbletranslate.app${path}`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
    env,
  );

describe("isLive / refusalFor", () => {
  it("keeps a cancelled licence live until its paid term ends", () => {
    expect(isLive(base({ status: "cancelled" }))).toBe(true);
  });

  it("refuses a refunded licence even if its term was not cut", () => {
    const licence = base({ status: "refunded" });
    expect(isLive(licence)).toBe(false);
    expect(refusalFor(licence, env)).toMatch(/refunded/);
  });

  it("allows the grace period after the term, then refuses", () => {
    expect(isLive(base({ expires_at: now() - GRACE_SECONDS + 60 }))).toBe(true);
    const ended = base({ status: "cancelled", expires_at: now() - GRACE_SECONDS - 1 });
    expect(isLive(ended)).toBe(false);
    expect(refusalFor(ended, env)).toMatch(/cancelled/);
  });
});

describe("keys and tokens", () => {
  it("makes keys without look-alike characters", () => {
    for (let i = 0; i < 200; i++) {
      expect(newLicenceKey()).toMatch(/^BT-[2-9A-HJKMNP-Z]{5}-[2-9A-HJKMNP-Z]{5}-[2-9A-HJKMNP-Z]{5}$/);
    }
  });

  it("never mints a token that outlives the term by more than the grace", async () => {
    const licence = base({ expires_at: now() + 86_400 });
    const claims = (await open(env, (await grantToken(env, licence, "dev1")).token))!;
    expect(claims.exp).toBe(licence.expires_at + GRACE_SECONDS);
  });

  it("caps a long term at the token TTL", async () => {
    const licence = base({ expires_at: now() + 365 * 86_400 });
    const claims = (await open(env, (await grantToken(env, licence, "dev1")).token))!;
    expect(claims.exp - claims.iat).toBe(TOKEN_TTL);
  });

  it("rejects a tampered token", async () => {
    const { token } = await grantToken(env, base(), "dev1");
    const [payload, sig] = token.split(".");
    // A character in the middle of the signature: the last one carries
    // padding bits, and changing only those would leave the bytes the same.
    const i = Math.floor(sig.length / 2);
    const bad = sig.slice(0, i) + (sig[i] === "A" ? "B" : "A") + sig.slice(i + 1);
    expect(await open(env, `${payload}.${bad}`)).toBeNull();
    // And the claims: raising the plan's limit must not survive the signature.
    const forged = btoa(JSON.stringify({ ...JSON.parse(atob(payload.replace(/-/g, "+").replace(/_/g, "/"))), lim: 999 }))
      .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    expect(await open(env, `${forged}.${sig}`)).toBeNull();
  });
});

describe("activate / refresh", () => {
  it("activates, and does not spend a second seat on the same machine", async () => {
    const { key, id } = await issueLicence(env, { provider: "paddle", cycle: "monthly" });
    expect((await post("/v1/activate", { key, device: "m1" })).status).toBe(200);
    expect((await post("/v1/activate", { key: key.toLowerCase(), device: "m1" })).status).toBe(200);
    const seats = await env.DB.prepare("SELECT COUNT(*) AS n FROM seats WHERE licence_id = ?")
      .bind(id)
      .first<{ n: number }>();
    expect(seats!.n).toBe(1);
  });

  it("holds the seat limit even when activations race", async () => {
    const { key } = await issueLicence(env, { provider: "paddle", cycle: "monthly", seats: 2 });
    const results = await Promise.all(
      ["a", "b", "c", "d"].map((device) => post("/v1/activate", { key, device })),
    );
    expect(results.filter((r) => r.status === 200)).toHaveLength(2);
    expect(results.filter((r) => r.status === 409)).toHaveLength(2);
  });

  it("refuses an unknown key", async () => {
    expect((await post("/v1/activate", { key: "BT-AAAAA-AAAAA-AAAAA", device: "m1" })).status).toBe(404);
  });

  it("refuses to refresh a token on a different machine", async () => {
    const { key } = await issueLicence(env, { provider: "paddle", cycle: "monthly" });
    const { token } = (await (await post("/v1/activate", { key, device: "m1" })).json()) as any;
    expect((await post("/v1/refresh", { token, device: "m1" })).status).toBe(200);
    expect((await post("/v1/refresh", { token, device: "m2" })).status).toBe(403);
  });

  it("stops refreshing once the licence is refunded", async () => {
    const { key, id } = await issueLicence(env, { provider: "paddle", cycle: "monthly" });
    const { token } = (await (await post("/v1/activate", { key, device: "m1" })).json()) as any;
    await endLicence(env, (await licenceById(env, id))!, "refunded");
    expect((await post("/v1/refresh", { token, device: "m1" })).status).toBe(403);
  });
});

describe("pages", () => {
  const get = (path: string) =>
    worker.fetch(new Request(`https://api.bubbletranslate.app${path}`), env);

  it("sends the welcome page only for a ref shaped like one we minted", async () => {
    const bad = await get("/welcome?ref=%3Cimg%20src%3Dx%3E");
    expect(bad.status).toBe(303);
    const good = await get(`/welcome?ref=${"a".repeat(32)}`);
    expect(good.status).toBe(200);
  });

  it("forbids framing and carries HSTS", async () => {
    const res = await get("/account");
    expect(res.headers.get("x-frame-options")).toBe("DENY");
    expect(res.headers.get("content-security-policy")).toContain("frame-ancestors 'none'");
    expect(res.headers.get("strict-transport-security")).toContain("max-age=");
    expect(res.headers.get("x-content-type-options")).toBe("nosniff");
  });
});
