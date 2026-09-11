// The Paddle mirror: what Paddle believes about a customer and a subscription,
// copied here from verified webhooks.
//
// This is a cache of someone else's records. The `licences` table is still
// what entitles anyone to anything -- the client checks a signed token minted
// from a licence, and no lookup here is on that path. What the mirror buys is
// two things the licence row cannot answer: which Paddle customer a licence
// key belongs to (so a portal session can be minted without the browser ever
// naming a customer id), and what Paddle intends to do next (so the account
// page can say "cancels on the 3rd" rather than "active" and nothing else).
//
// Every write goes through this file, and every write is an upsert keyed on
// the Paddle id. Deliveries are at-least-once and unordered, so a handler that
// inserted blindly would either fail on the second delivery or overwrite a
// current status with a stale one.

import type { Env } from "./env";
import { now } from "./tokens";

export interface PaddleCustomer {
  customer_id: string;
  email: string | null;
  status: string | null;
}

export interface PaddleSubscription {
  subscription_id: string;
  customer_id: string;
  status: string;
  price_id: string | null;
  product_id: string | null;
  scheduled_change_action: string | null;
  scheduled_change_at: string | null;
  next_billed_at: string | null;
}

/** The event's own `occurred_at`, in unix seconds.
 *
 *  Not the arrival time: two events can arrive in the wrong order, and the
 *  question the upserts ask is which one Paddle considers later. A payload
 *  without a usable timestamp falls back to 0, which makes it lose every
 *  comparison rather than win one it should not. */
export function occurredAt(event: { occurredAt?: string | null }): number {
  const parsed = Date.parse(event.occurredAt ?? "");
  return Number.isFinite(parsed) ? Math.floor(parsed / 1000) : 0;
}

// -- writing -----------------------------------------------------------------

/** Records a customer. `customer.created` and `customer.updated` are the same
 *  write: the second delivery of a create and the first of an update are
 *  indistinguishable, and both should leave the row saying the same thing. */
export async function mirrorCustomer(env: Env, data: any, eventAt: number): Promise<void> {
  const id = data?.id ? String(data.id) : null;
  if (!id) return;

  await env.DB.prepare(
    `INSERT INTO paddle_customers (customer_id, email, status, event_at, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT (customer_id) DO UPDATE SET
       email      = excluded.email,
       status     = excluded.status,
       event_at   = excluded.event_at,
       updated_at = excluded.updated_at
     WHERE excluded.event_at >= paddle_customers.event_at`,
  )
    .bind(
      id,
      data?.email ? String(data.email) : null,
      data?.status ? String(data.status) : null,
      eventAt,
      now(),
      now(),
    )
    .run();
}

/** Records a subscription. Used by `subscription.created`, `.updated` and
 *  `.canceled` alike -- the payload is the same entity in all three, and the
 *  status field is what differs, so there is nothing for three code paths to
 *  disagree about. */
export async function mirrorSubscription(env: Env, data: any, eventAt: number): Promise<void> {
  const id = data?.id ? String(data.id) : null;
  const customerId = data?.customer_id ? String(data.customer_id) : null;
  const status = data?.status ? String(data.status) : null;
  if (!id || !customerId || !status) {
    console.error(`Paddle subscription event missing id, customer or status: ${id ?? "?"}`);
    return;
  }

  // The first item is the plan. This service sells exactly one thing, so a
  // subscription with more than one line is not something the mirror can
  // usefully flatten -- it takes the first and leaves the rest to Paddle.
  const item = (data?.items ?? [])[0] ?? {};
  const change = data?.scheduled_change ?? null;

  await env.DB.prepare(
    `INSERT INTO paddle_subscriptions
       (subscription_id, customer_id, status, price_id, product_id,
        scheduled_change_action, scheduled_change_at, next_billed_at,
        event_at, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (subscription_id) DO UPDATE SET
       customer_id             = excluded.customer_id,
       status                  = excluded.status,
       price_id                = excluded.price_id,
       product_id              = excluded.product_id,
       scheduled_change_action = excluded.scheduled_change_action,
       scheduled_change_at     = excluded.scheduled_change_at,
       next_billed_at          = excluded.next_billed_at,
       event_at                = excluded.event_at,
       updated_at              = excluded.updated_at
     WHERE excluded.event_at >= paddle_subscriptions.event_at`,
  )
    .bind(
      id,
      customerId,
      status,
      item?.price?.id ? String(item.price.id) : null,
      item?.price?.product_id ? String(item.price.product_id) : null,
      change?.action ? String(change.action) : null,
      change?.effective_at ? String(change.effective_at) : null,
      data?.next_billed_at ? String(data.next_billed_at) : null,
      eventAt,
      now(),
      now(),
    )
    .run();
}

// -- reading -----------------------------------------------------------------

export const subscriptionById = (env: Env, id: string) =>
  env.DB.prepare(
    `SELECT subscription_id, customer_id, status, price_id, product_id,
            scheduled_change_action, scheduled_change_at, next_billed_at
       FROM paddle_subscriptions WHERE subscription_id = ?`,
  )
    .bind(id)
    .first<PaddleSubscription>();

export const customerById = (env: Env, id: string) =>
  env.DB.prepare(
    "SELECT customer_id, email, status FROM paddle_customers WHERE customer_id = ?",
  )
    .bind(id)
    .first<PaddleCustomer>();

/** Whether a mirrored subscription currently pays for anything.
 *
 *  Two rules, and both are easy to get wrong in the expensive direction:
 *
 *  A `scheduled_change` does not revoke anything. It says what Paddle will do
 *  at the end of the period the customer has already paid for; reading it as
 *  "cancelled" takes back time they bought the moment they ask to leave, which
 *  is the same mistake `endLicence("cancelled")` exists to avoid.
 *
 *  `trialing` grants access, because a trial is access -- that is what it is
 *  for. Only `status` actually being `canceled` revokes.
 *
 *  `past_due` keeps access here on purpose: Paddle is still retrying the card,
 *  the customer usually has a working licence and a temporarily dead payment
 *  method, and locking them out mid-dunning is how a recoverable failed charge
 *  becomes a support ticket and a chargeback. `paused` does not grant, because
 *  a pause is the customer asking not to be billed and not to be served.
 *  Either way the licence's own term is the backstop -- see `isLive`. */
export function grantsAccess(subscription: Pick<PaddleSubscription, "status">): boolean {
  switch (subscription.status) {
    case "active":
    case "trialing":
    case "past_due":
      return true;
    default:
      return false;
  }
}

/** Records a customer id and address seen on some other event -- a completed
 *  transaction, say -- without claiming to know the rest of the customer.
 *
 *  Insert-only. A `customer.created` payload is the authority on a customer
 *  row; this is a sighting, and a sighting must never overwrite the record. It
 *  exists so that a deployment subscribed only to the transaction and
 *  subscription events still learns the customer id it needs to open the
 *  portal. */
export async function noteCustomer(
  env: Env,
  customerId: string | null,
  email: string | null,
): Promise<void> {
  if (!customerId) return;
  await env.DB.prepare(
    `INSERT INTO paddle_customers (customer_id, email, status, event_at, created_at, updated_at)
     VALUES (?, ?, NULL, 0, ?, ?)
     ON CONFLICT (customer_id) DO NOTHING`,
  )
    .bind(customerId, email, now(), now())
    .run();
}
