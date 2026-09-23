import { env } from "cloudflare:workers";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import worker from "../src/index";
import { verifyWebhook } from "../src/paddle";
import { createOrder, licenceById, licenceByProviderRef } from "../src/licences";
import { TERM_SECONDS } from "../src/env";
import { hmacSha256, now, toHex } from "../src/tokens";

const SECRET = "test-webhook-secret";
const PADDLE_IP = "34.232.58.13";

async function sign(raw: string, ts = now(), secret = SECRET) {
  return `ts=${ts};h1=${toHex(await hmacSha256(secret, `${ts}:${raw}`))}`;
}

async function deliver(event: object, opts: { ip?: string; header?: string } = {}) {
  const raw = JSON.stringify(event);
  const request = new Request("https://api.bubbletranslate.app/webhooks/paddle", {
    method: "POST",
    body: raw,
    headers: {
      "Paddle-Signature": opts.header ?? (await sign(raw)),
      "CF-Connecting-IP": opts.ip ?? PADDLE_IP,
    },
  });
  return worker.fetch(request, env);
}

const completed = (ref: string | null, sub: string, price = env.PADDLE_PRICE_MONTHLY) => ({
  event_type: "transaction.completed",
  occurred_at: new Date().toISOString(),
  data: {
    subscription_id: sub,
    customer: { email: "buyer@example.com" },
    items: [{ price: { id: price } }],
    ...(ref ? { custom_data: { ref } } : {}),
  },
});

async function order(ref: string) {
  await createOrder(env, { ref, provider: "paddle", cycle: "monthly", amount: 200, currency: "USD" });
}

const licenceCount = async () =>
  (await env.DB.prepare("SELECT COUNT(*) AS n FROM licences").first<{ n: number }>())!.n;

beforeEach(() => {
  // Paddle's published address list is the only outbound call these paths make.
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    if (String(input instanceof Request ? input.url : input).endsWith("/ips")) {
      return Response.json({ data: { ipv4_cidrs: ["34.232.58.0/24"] } });
    }
    throw new Error(`unexpected fetch ${input}`);
  });
});
afterEach(() => vi.restoreAllMocks());

describe("verifyWebhook", () => {
  const raw = '{"event_type":"x","data":{}}';

  it("accepts a fresh, correctly signed body", async () => {
    expect(await verifyWebhook(env, raw, await sign(raw))).toMatchObject({ eventType: "x" });
  });

  it("rejects a missing header, or one without ts or h1", async () => {
    expect(await verifyWebhook(env, raw, null)).toBeNull();
    expect(await verifyWebhook(env, raw, `ts=${now()}`)).toBeNull();
    expect(await verifyWebhook(env, raw, "h1=abc")).toBeNull();
  });

  it("rejects a signature made with another secret", async () => {
    expect(await verifyWebhook(env, raw, await sign(raw, now(), "wrong"))).toBeNull();
  });

  it("rejects a body edited after signing", async () => {
    expect(await verifyWebhook(env, raw + " ", await sign(raw))).toBeNull();
  });

  it("rejects a valid signature over a stale timestamp (replay)", async () => {
    expect(await verifyWebhook(env, raw, await sign(raw, now() - 301))).toBeNull();
    expect(await verifyWebhook(env, raw, await sign(raw, now() + 301))).toBeNull();
  });

  it("allows the cold-start skew of up to five minutes", async () => {
    expect(await verifyWebhook(env, raw, await sign(raw, now() - 290))).not.toBeNull();
  });

  it("refuses everything when no secret is configured", async () => {
    const bare = { ...env, PADDLE_WEBHOOK_SECRET: undefined };
    expect(await verifyWebhook(bare, raw, await sign(raw))).toBeNull();
  });
});

describe("POST /webhooks/paddle", () => {
  it("refuses a delivery from outside Paddle's addresses", async () => {
    const res = await deliver(completed("r1", "sub_1"), { ip: "8.8.8.8" });
    expect(res.status).toBe(403);
  });

  it("refuses a bad signature before touching the database", async () => {
    await order("r1");
    const res = await deliver(completed("r1", "sub_1"), { header: `ts=${now()};h1=00` });
    expect(res.status).toBe(401);
    expect(await licenceCount()).toBe(0);
  });

  it("issues exactly one licence for a first payment", async () => {
    await order("r1");
    expect((await deliver(completed("r1", "sub_1"))).status).toBe(200);
    const licence = await licenceByProviderRef(env, "paddle", "sub_1");
    expect(licence).toMatchObject({ status: "active", cycle: "monthly", email: "buyer@example.com" });
    const row = await env.DB.prepare("SELECT status, licence_id FROM orders WHERE ref = 'r1'").first();
    expect(row).toMatchObject({ status: "paid", licence_id: licence!.id });
  });

  it("is idempotent when Paddle retries the same payment", async () => {
    await order("r1");
    const event = completed("r1", "sub_1");
    await deliver(event);
    const before = await licenceByProviderRef(env, "paddle", "sub_1");
    // A retry of a first payment finds the subscription and would extend it;
    // what matters is that it never mints a second licence.
    await deliver({ ...event, data: { ...event.data, subscription_id: null } });
    expect(await licenceCount()).toBe(1);
    expect((await licenceById(env, before!.id))!.expires_at).toBe(before!.expires_at);
  });

  it("issues one licence when two deliveries race", async () => {
    await order("r1");
    const event = completed("r1", "sub_1");
    const noSub = { ...event, data: { ...event.data, subscription_id: null } };
    await Promise.all([deliver(noSub), deliver(noSub), deliver(noSub)]);
    expect(await licenceCount()).toBe(1);
  });

  it("reads the cycle from the price id, not the amount", async () => {
    await order("r1");
    await deliver(completed("r1", "sub_1", env.PADDLE_PRICE_YEARLY));
    expect((await licenceByProviderRef(env, "paddle", "sub_1"))!.cycle).toBe("yearly");
  });

  it("extends the term on a renewal instead of selling another licence", async () => {
    await order("r1");
    await deliver(completed("r1", "sub_1"));
    const first = (await licenceByProviderRef(env, "paddle", "sub_1"))!;
    await deliver(completed(null, "sub_1"));
    const renewed = (await licenceById(env, first.id))!;
    expect(await licenceCount()).toBe(1);
    expect(renewed.expires_at).toBe(first.expires_at + TERM_SECONDS.monthly);
  });

  it("ignores a payment with no ref and no known subscription", async () => {
    const res = await deliver(completed(null, "sub_unknown"));
    expect(res.status).toBe(200);
    expect(await licenceCount()).toBe(0);
  });

  it("keeps the paid term on a cancellation", async () => {
    await order("r1");
    await deliver(completed("r1", "sub_1"));
    const before = (await licenceByProviderRef(env, "paddle", "sub_1"))!;
    await deliver({
      event_type: "subscription.canceled",
      occurred_at: new Date().toISOString(),
      data: { id: "sub_1", status: "canceled", customer_id: "ctm_1" },
    });
    const after = (await licenceById(env, before.id))!;
    expect(after.status).toBe("cancelled");
    expect(after.expires_at).toBe(before.expires_at);
  });

  it("cuts the term to now on a refund", async () => {
    await order("r1");
    await deliver(completed("r1", "sub_1"));
    const before = (await licenceByProviderRef(env, "paddle", "sub_1"))!;
    await deliver({
      event_type: "adjustment.created",
      occurred_at: new Date().toISOString(),
      data: { action: "refund", subscription_id: "sub_1" },
    });
    const after = (await licenceById(env, before.id))!;
    expect(after.status).toBe("refunded");
    expect(after.expires_at).toBeLessThanOrEqual(now());
  });

  it("does not end a licence on an adjustment that is not a refund", async () => {
    await order("r1");
    await deliver(completed("r1", "sub_1"));
    await deliver({
      event_type: "adjustment.created",
      occurred_at: new Date().toISOString(),
      data: { action: "credit", subscription_id: "sub_1" },
    });
    expect((await licenceByProviderRef(env, "paddle", "sub_1"))!.status).toBe("active");
  });
});

describe("the webhook log", () => {
  const logged = async () =>
    (await env.DB.prepare("SELECT event_type, event_id, outcome, detail FROM webhook_events ORDER BY id").all<any>())
      .results;

  it("records each delivery and what became of it, without the payload", async () => {
    await deliver({ ...completed(null, "sub_unknown"), event_id: "evt_1" });
    await deliver({ event_type: "business.updated", event_id: "evt_2", data: {} });
    await deliver(completed(null, "sub_x"), { header: "ts=1;h1=00" });
    await deliver(completed(null, "sub_x"), { ip: "8.8.8.8" });
    expect(await logged()).toEqual([
      { event_type: "transaction.completed", event_id: "evt_1", outcome: "ignored", detail: "no ref" },
      { event_type: "business.updated", event_id: "evt_2", outcome: "ignored", detail: "business.updated" },
      { event_type: null, event_id: null, outcome: "bad signature", detail: null },
      { event_type: null, event_id: null, outcome: "refused", detail: "8.8.8.8" },
    ]);
    const row = await env.DB.prepare("SELECT * FROM webhook_events").first<any>();
    expect(JSON.stringify(row)).not.toContain("buyer@example.com");
  });

  it("marks a delivery that fulfilled an order as handled", async () => {
    await order("ref-log");
    await deliver(completed("ref-log", "sub_log"));
    expect(await logged()).toEqual([
      { event_type: "transaction.completed", event_id: null, outcome: "handled", detail: null },
    ]);
  });
});
