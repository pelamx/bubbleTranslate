// The Paddle balance, rebuilt from Paddle's own figures: every completed
// transaction, what Paddle took as tax and fee, what it left as earnings, and
// what refunds and chargebacks took back. Read live from the API on each panel
// load rather than mirrored, so the numbers are Paddle's to the cent.
//
// Paddle's API has no balance or payout endpoint, so this is the balance
// before payouts: the sum of earnings, less what adjustments removed. Until
// the first payout it is exactly the "Total balance" on the dashboard.

import { type Env, paddleApiBase } from "../env";

export interface BalanceRow {
  transaction_id: string;
  /** Unix seconds Paddle billed it. */
  at: number;
  /** `web` for a checkout, `subscription_recurring` for a renewal. */
  origin: string | null;
  subscription_id: string | null;
  email: string | null;
  country: string | null;
  /** The transaction's own currency: what the buyer saw. */
  currency: string;
  paid: number;
  tax: number;
  /** The balance currency, which everything below is in. */
  balance_currency: string;
  fee: number;
  earnings: number;
  /** Earnings reversed by refunds and chargebacks. */
  refunded: number;
  /** Paddle's fee, kept on a refund and charged to the balance. */
  retained_fee: number;
  chargeback_fee: number;
  /** What this transaction leaves in the balance. */
  net: number;
}

export interface Balance {
  rows: BalanceRow[];
  currency: string;
  paid: number;
  tax: number;
  fee: number;
  earnings: number;
  refunded: number;
  retained_fee: number;
  chargeback_fee: number;
  balance: number;
  /** True when there were more transactions than were read. */
  truncated: boolean;
}

const int = (v: unknown) => {
  const n = Number(v);
  return Number.isFinite(n) ? Math.round(n) : 0;
};

/** One transaction as Paddle returns it, with `customer`, `address` and
 *  `adjustments_totals` included, turned into a row. */
export function toRow(t: any): BalanceRow {
  const totals = t?.details?.totals ?? {};
  const payout = t?.details?.payout_totals ?? totals;
  const adj = t?.adjustments_totals ?? {};
  const cb = t?.details?.adjusted_payout_totals?.chargeback_fee?.amount;
  const earnings = int(payout.earnings);
  const refunded = int(adj.earnings);
  const retained = int(adj.retained_fee);
  const chargebackFee = int(cb);
  const billed = Date.parse(t?.billed_at ?? t?.created_at ?? "");
  return {
    transaction_id: String(t?.id ?? ""),
    at: Number.isFinite(billed) ? Math.floor(billed / 1000) : 0,
    origin: t?.origin ? String(t.origin) : null,
    subscription_id: t?.subscription_id ? String(t.subscription_id) : null,
    email: t?.customer?.email ? String(t.customer.email) : null,
    country: t?.address?.country_code ? String(t.address.country_code) : null,
    currency: String(totals.currency_code ?? t?.currency_code ?? "USD"),
    paid: int(totals.grand_total ?? totals.total),
    tax: int(totals.tax),
    balance_currency: String(payout.currency_code ?? totals.currency_code ?? "USD"),
    fee: int(payout.fee),
    earnings,
    refunded,
    retained_fee: retained,
    chargeback_fee: chargebackFee,
    net: earnings - refunded - retained - chargebackFee,
  };
}

/** The rows newest first, with the totals. There is no running balance per
 *  row: a refund is counted against the sale it undoes, and Paddle does not
 *  say on the transaction when the refund happened, so a running figure would
 *  show balances that never existed. Amounts in a currency other than the balance
 *  currency are only ever the buyer-facing `paid` and `tax`, so those two are
 *  summed only where they match. */
export function summarise(rows: BalanceRow[], truncated = false): Balance {
  const sorted = [...rows].sort((a, b) => b.at - a.at || b.transaction_id.localeCompare(a.transaction_id));
  const currency = sorted[0]?.balance_currency ?? "USD";
  const out: Balance = {
    rows: [], currency, paid: 0, tax: 0, fee: 0, earnings: 0, refunded: 0,
    retained_fee: 0, chargeback_fee: 0, balance: 0, truncated,
  };
  for (const r of sorted) {
    out.balance += r.net;
    if (r.currency === currency) {
      out.paid += r.paid;
      out.tax += r.tax;
    }
    out.fee += r.fee;
    out.earnings += r.earnings;
    out.refunded += r.refunded;
    out.retained_fee += r.retained_fee;
    out.chargeback_fee += r.chargeback_fee;
  }
  out.rows = sorted;
  return out;
}

/** How many pages of 30 to read: 600 transactions, well inside a Worker's
 *  subrequest allowance. Past that the card says it is incomplete. */
const MAX_PAGES = 20;

/** Every completed transaction from Paddle, as a balance. A string is the
 *  reason it could not be read, for the card to show. */
export async function paddleBalance(env: Env): Promise<Balance | string> {
  if (!env.PADDLE_API_KEY) return "No Paddle API key is configured.";
  const rows: BalanceRow[] = [];
  let after: string | null = null;
  try {
    for (let page = 0; page < MAX_PAGES; page++) {
      const url = new URL(`${paddleApiBase(env)}/transactions`);
      url.searchParams.set("status", "completed");
      url.searchParams.set("include", "customer,address,adjustments_totals");
      url.searchParams.set("order_by", "id[ASC]");
      url.searchParams.set("per_page", "30");
      if (after) url.searchParams.set("after", after);
      const response = await fetch(url, {
        headers: { authorization: `Bearer ${env.PADDLE_API_KEY}` },
        signal: AbortSignal.timeout(8000),
      });
      if (!response.ok) {
        return response.status === 403
          ? "Paddle refused: the API key needs the transaction.read permission."
          : `Paddle answered ${response.status}.`;
      }
      const body: any = await response.json();
      const data: any[] = Array.isArray(body?.data) ? body.data : [];
      for (const t of data) rows.push(toRow(t));
      if (!body?.meta?.pagination?.has_more || !data.length) return summarise(rows);
      after = String(data[data.length - 1].id);
    }
    return summarise(rows, true);
  } catch (err) {
    console.error("could not read the Paddle balance", err);
    return "Paddle could not be reached just now.";
  }
}
