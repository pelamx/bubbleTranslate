import { env } from "cloudflare:workers";
import { afterEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { paddleBalance, summarise, toRow } from "../src/admin/balance";

// The live account on 2026-10-14: one sale from Australia, four test
// purchases refunded. Paddle's dashboard said $13.38.
const txn = (id: string, billed: string, paid: number, tax: number, fee: number, refunded: boolean, extra = {}) => ({
  id,
  billed_at: billed,
  origin: "web",
  subscription_id: `sub_${id}`,
  currency_code: "USD",
  customer: { email: `${id}@example.com` },
  address: { country_code: "TR" },
  details: {
    totals: { grand_total: String(paid), tax: String(tax), currency_code: "USD" },
    payout_totals: { fee: String(fee), earnings: String(paid - tax - fee), currency_code: "USD" },
  },
  adjustments_totals: refunded
    ? { earnings: String(paid - tax - fee), retained_fee: String(fee) }
    : { earnings: "0", retained_fee: "0" },
  ...extra,
});
const LIVE = [
  txn("t1", "2026-09-11T08:28:00Z", 200, 33, 60, true),
  txn("t2", "2026-09-19T09:43:00Z", 2000, 333, 150, true),
  txn("t3", "2026-10-11T06:54:00Z", 2000, 182, 150, false, { address: { country_code: "AU" } }),
  txn("t4", "2026-10-14T11:06:00Z", 200, 33, 60, true),
  txn("t5", "2026-10-14T11:30:00Z", 200, 33, 60, true),
];

afterEach(() => vi.restoreAllMocks());

it("rebuilds the dashboard balance to the cent", () => {
  const b = summarise(LIVE.map(toRow));
  expect(b.balance).toBe(1338);
  expect(b.earnings).toBe(3506);
  expect(b.fee).toBe(480);
  expect(b.retained_fee).toBe(330);
  expect(b.paid).toBe(4600);
  // Newest first; the nets add up to the balance.
  expect(b.rows.map((r) => r.transaction_id)).toEqual(["t5", "t4", "t3", "t2", "t1"]);
  expect(b.rows.find((r) => r.transaction_id === "t3")).toMatchObject({ net: 1668, country: "AU" });
  expect(b.rows[0]).toMatchObject({ net: -60, retained_fee: 60 });
  expect(b.rows.reduce((a, r) => a + r.net, 0)).toBe(1338);
});

it("reads every page from Paddle", async () => {
  const spy = vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    const after = new URL(String(input)).searchParams.get("after");
    const page = after ? LIVE.slice(3) : LIVE.slice(0, 3);
    return Response.json({ data: page, meta: { pagination: { has_more: !after } } });
  });
  const b = await paddleBalance({ ...env, PADDLE_API_KEY: "k" } as any);
  expect(spy).toHaveBeenCalledTimes(2);
  expect(typeof b === "string" ? b : b.balance).toBe(1338);
});

it("passes on Paddle's own reason when it refuses the key", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({ error: { code: "forbidden", detail: "You aren't permitted to perform this request." } }, { status: 403 }),
  );
  expect(await paddleBalance({ ...env, PADDLE_API_KEY: "k" } as any)).toBe(
    "Paddle answered 403 (forbidden): You aren't permitted to perform this request.",
  );
});

it("still gives the figures when the key may not read customers", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) =>
    new URL(String(input)).searchParams.get("include")!.includes("customer")
      ? Response.json({ error: { code: "forbidden", detail: "no customer.read" } }, { status: 403 })
      : Response.json({
          data: LIVE.map(({ customer, address, ...t }) => t),
          meta: { pagination: { has_more: false } },
        }),
  );
  const b = await paddleBalance({ ...env, PADDLE_API_KEY: "k" } as any);
  if (typeof b === "string") throw new Error(b);
  expect(b.balance).toBe(1338);
  expect(b.rows[0].email).toBeNull();
  expect(b.note).toContain("customer.read");
});

it("shows the balance card on the panel", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) =>
    String(input).includes("/transactions")
      ? Response.json({ data: LIVE, meta: { pagination: { has_more: false } } })
      : new Response("", { status: 503 }),
  );
  const res = await worker.fetch(
    new Request("https://api.bubbletranslate.app/admin", {
      headers: { authorization: `Basic ${btoa("op:letmein")}` },
    }),
    { ...env, ADMIN_PASSWORD: "letmein", PADDLE_API_KEY: "k" } as any,
  );
  const html = await res.text();
  expect(html).toContain("Paddle balance — $13.38");
  expect(html).toContain("Paddle fees <b>$4.80</b> (+$3.30 kept on refunds)");
  expect(html).toContain("t3@example.com");
});
