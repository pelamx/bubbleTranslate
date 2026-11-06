import { env } from "cloudflare:workers";
import { afterEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { base58, formatPlx, plxReference, received } from "../src/plx";

const MINT = "PLXmint1111111111111111111111111111111111111";
const TREASURY = "Treasury11111111111111111111111111111111111";
const plxEnv: any = { ...env, PLX_MINT: MINT, PLX_TREASURY: TREASURY, PLX_USD: "0.5", SOLANA_RPC_URL: "https://rpc.test" };

const call = (path: string, init?: RequestInit, e: any = plxEnv) =>
  worker.fetch(new Request(`https://api.bubbletranslate.app${path}`, init), e);

const checkout = (cycle: string, e: any = plxEnv) =>
  call(
    "/checkout/plx",
    { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cycle, email: "a@b.co" }) },
    e,
  );

/** A finalized transaction that moved `amount` base units of `mint` into `owner`. */
const tx = (amount: bigint, owner = TREASURY, mint = MINT) => ({
  meta: {
    err: null,
    preTokenBalances: [{ owner, mint, uiTokenAmount: { amount: "1000" } }],
    postTokenBalances: [{ owner, mint, uiTokenAmount: { amount: String(1000n + amount) } }],
  },
});

/** Answers the two RPC calls with one signature and the given transaction. */
const chain = (transaction: unknown) =>
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_input, init) => {
    const { method } = JSON.parse(String(init?.body));
    return Response.json({
      jsonrpc: "2.0",
      id: 1,
      result: method === "getSignaturesForAddress" ? [{ signature: "sig1", err: null }] : transaction,
    });
  });

afterEach(() => vi.restoreAllMocks());

it("encodes base58 and formats PLX the way wallets read them", () => {
  expect(base58(new Uint8Array(32))).toBe("11111111111111111111111111111111");
  expect(base58(new TextEncoder().encode("Hello World!"))).toBe("2NEpo7TZRRrLZSi2U");
  expect(formatPlx(40_000_000_000)).toBe("40");
  expect(formatPlx(1_500_000_001)).toBe("1.500000001");
});

it("counts only what reached the treasury in PLX", () => {
  expect(received(tx(5n), TREASURY, MINT)).toBe(5n);
  expect(received(tx(5n, "someone-else"), TREASURY, MINT)).toBe(0n);
  expect(received(tx(5n, TREASURY, "other-mint"), TREASURY, MINT)).toBe(0n);
});

it("is not offered, and refuses an order, without a mint, treasury and rate", async () => {
  expect(await (await call("/buy", undefined, env)).text()).not.toContain('id="plx"');
  expect((await checkout("yearly", env)).status).toBe(503);
  expect(await (await call("/buy")).text()).toContain('id="plx"');
});

it("quotes the dollar price in PLX and issues Pro once the chain shows it paid", async () => {
  const created: any = await (await checkout("yearly")).json();
  // $20 at $0.50 a PLX.
  expect(created.plx).toBe("40");
  expect(created.url).toBe(
    `solana:${TREASURY}?amount=40&spl-token=${MINT}&reference=${await plxReference(created.ref)}` +
      "&label=bubbleTranslate&message=bubbleTranslate+Pro",
  );

  chain(null);
  expect(await (await call(`/v1/plx/${created.ref}`)).json()).toEqual({ status: "pending" });

  vi.restoreAllMocks();
  chain(tx(40_000_000_000n));
  expect(await (await call(`/v1/plx/${created.ref}`)).json()).toEqual({ status: "paid" });
  expect(await (await call(`/v1/plx/${created.ref}`)).json()).toEqual({ status: "paid" });

  const licences = await env.DB.prepare("SELECT provider, provider_ref, cycle FROM licences").all();
  expect(licences.results).toEqual([{ provider: "plx", provider_ref: "sig1", cycle: "yearly" }]);
  const order: any = await (await call(`/v1/order/${created.ref}`)).json();
  expect(order.status).toBe("paid");
  expect(order.key).toBeTruthy();
});

it("does not pay for an order with too little PLX, or with a transaction already used", async () => {
  const short: any = await (await checkout("monthly")).json();
  chain(tx(3_999_999_999n));
  expect(await (await call(`/v1/plx/${short.ref}`)).json()).toEqual({ status: "pending" });

  vi.restoreAllMocks();
  const first: any = await (await checkout("monthly")).json();
  const second: any = await (await checkout("monthly")).json();
  // One transaction that names both orders' references pays only one of them.
  chain(tx(4_000_000_000n));
  expect(await (await call(`/v1/plx/${first.ref}`)).json()).toEqual({ status: "paid" });
  expect(await (await call(`/v1/plx/${second.ref}`)).json()).toEqual({ status: "pending" });
  expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM licences").first<any>()).n).toBe(1);
});
