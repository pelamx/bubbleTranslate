// Pro paid in PLX, the Solana token at pelamx.com.
//
// The buyer pays from their own wallet through a Solana Pay link. Nothing is
// held here: no wallet, no private key, no balance. Each order gets a
// reference derived from its ref, the wallet puts that reference in the
// transfer, and the chain then says which order a payment was for. This file
// only reads it back.

import { type Cycle, type Env, USD_AMOUNT } from "./env";

/** PLX's decimals, from the token's own metadata (Token-2022, 9 decimals). */
const DECIMALS = 9;
const UNIT = 10 ** DECIMALS;

const rate = (env: Env): number | null => {
  const r = Number(env.PLX_USD);
  return Number.isFinite(r) && r > 0 ? r : null;
};

/** Base units of PLX for a cycle's dollar price, rounded up so a payment is
 *  never a fraction short.
 *
 *  shortcut: the rate is the fixed PLX_USD var, because PLX has no market pool
 *  to price it from yet; read it from Jupiter once a pool exists. */
export function plxQuote(env: Env, cycle: Cycle): number {
  return Math.ceil((USD_AMOUNT[cycle] * UNIT) / (rate(env) ?? NaN));
}

/** Whether the buy page may offer PLX. A missing mint, treasury, RPC or rate
 *  is "not offered", never a price of zero; so is a rate low enough that the
 *  quote stops being an exact integer. */
export function plxConfigured(env: Env): boolean {
  return Boolean(
    env.PLX_MINT?.trim() &&
      env.PLX_TREASURY?.trim() &&
      env.SOLANA_RPC_URL?.trim() &&
      Number.isSafeInteger(plxQuote(env, "yearly")),
  );
}

/** Base units as the decimal a wallet shows, without trailing zeros. */
export function formatPlx(units: number): string {
  const frac = String(units % UNIT).padStart(DECIMALS, "0").replace(/0+$/, "");
  return frac ? `${Math.floor(units / UNIT)}.${frac}` : String(Math.floor(units / UNIT));
}

const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

export function base58(bytes: Uint8Array): string {
  let n = 0n;
  for (const b of bytes) n = n * 256n + BigInt(b);
  let out = "";
  while (n > 0n) {
    out = ALPHABET[Number(n % 58n)] + out;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b) break;
    out = "1" + out;
  }
  return out;
}

/** The order's Solana Pay reference: 32 bytes, as a public key is. Hashed
 *  rather than the ref itself, because the reference is public on-chain and
 *  the ref is what unlocks the licence key on the success page. */
export async function plxReference(ref: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`plx:${ref}`));
  return base58(new Uint8Array(digest));
}

/** The Solana Pay transfer request a wallet opens or scans. */
export async function plxPayUrl(env: Env, ref: string, units: number): Promise<string> {
  const q = new URLSearchParams({
    amount: formatPlx(units),
    "spl-token": env.PLX_MINT!.trim(),
    reference: await plxReference(ref),
    label: "bubbleTranslate",
    message: "bubbleTranslate Pro",
  });
  return `solana:${env.PLX_TREASURY!.trim()}?${q}`;
}

async function rpc(env: Env, method: string, params: unknown[]): Promise<any> {
  const res = await fetch(env.SOLANA_RPC_URL!.trim(), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const body: any = await res.json();
  if (body?.error) throw new Error(`solana ${method}: ${body.error.message ?? "error"}`);
  return body?.result;
}

/** How much PLX a transaction moved into the treasury's token accounts. Read
 *  from the balances before and after rather than from the instructions, so a
 *  plain transfer, a checked one and a Token-2022 one with a transfer fee all
 *  count only what actually arrived. */
export function received(tx: any, treasury: string, mint: string): bigint {
  const sum = (list: any[] | undefined) =>
    (list ?? [])
      .filter((b) => b?.owner === treasury && b?.mint === mint)
      .reduce((s: bigint, b) => s + BigInt(b?.uiTokenAmount?.amount ?? 0), 0n);
  return sum(tx?.meta?.postTokenBalances) - sum(tx?.meta?.preTokenBalances);
}

/** The signature of a finalized transaction that carries this order's
 *  reference and paid the treasury at least `units`, or null. Finalized, not
 *  confirmed: a licence issued on a payment that is later dropped cannot be
 *  taken back. */
export async function findPlxPayment(env: Env, ref: string, units: number): Promise<string | null> {
  const treasury = env.PLX_TREASURY!.trim();
  const mint = env.PLX_MINT!.trim();
  const sigs = await rpc(env, "getSignaturesForAddress", [
    await plxReference(ref),
    { limit: 20, commitment: "finalized" },
  ]);
  for (const s of Array.isArray(sigs) ? sigs : []) {
    if (s?.err || typeof s?.signature !== "string") continue;
    const tx = await rpc(env, "getTransaction", [
      s.signature,
      { commitment: "finalized", encoding: "jsonParsed", maxSupportedTransactionVersion: 0 },
    ]);
    if (tx && !tx.meta?.err && received(tx, treasury, mint) >= BigInt(units)) return s.signature;
  }
  return null;
}
