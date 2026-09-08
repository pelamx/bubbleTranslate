# bubbleTranslate

A bubble translator for macOS and Linux. Select text in any application and a
small bubble appears at the cursor with the translation.

- `src/` — the Rust client (egui bubble, selection monitor, translation chain,
  licence check, daily allowance).
- `service/` — the licence service, a Cloudflare Worker on D1. It sells Pro,
  issues licence keys and signs entitlement tokens. It is **never** on the path
  of a translation.
- The website lives in a separate repository, `pelamx/bubbletranslate.app`.

Run `cargo test` before proposing a client change. The suite includes an
integration test that drives the real binary, so a change to startup ordering
is caught rather than assumed safe.

## Paddle

Paddle is how everywhere outside Turkey pays. Turkey goes through PayTR; see
`service/src/paytr.ts`. Paddle is the **merchant of record**: it sells the
licence to the customer and we sell it to Paddle, which is why VAT and
invoicing are Paddle's problem rather than ours in every country at once.

### Which SDK to use: none

**Do not add `@paddle/paddle-node-sdk` or any other server SDK.** The service
is a Cloudflare Worker, not Node. Talk to Paddle with `fetch`, and do crypto
with WebCrypto — that is what `service/src/tokens.ts` already wraps.

The browser side is the exception: checkout is **Paddle.js v2**, loaded from
`https://cdn.paddle.com/paddle/v2/paddle.js` in `service/src/pages.ts`. No card
number and no billing address ever reaches our Worker; all we get is a signed
webhook saying a payment happened. Keep it that way.

The `paddle` plugin's skills are worth reading, but they are written for
**Next.js Route Handlers and Server Actions using `@paddle/paddle-node-sdk`**.
None of that applies here. Take the Paddle semantics from them and ignore the
scaffolding: no `npm install`, no SDK singleton, no `Environment` enum. This
section wins over a skill wherever the two disagree.

### Environments

`PADDLE_ENV` is the single switch. It is `"sandbox"` until it is
`"production"`, and it chooses **both** the API host and the Paddle.js
environment, so the two cannot disagree.

- Never hardcode a Paddle host. Call `paddleApiBase(env)` from
  `service/src/env.ts`, which returns `api.paddle.com` in production and
  `sandbox-api.paddle.com` otherwise.
- Never hardcode a price id. Call `paddlePriceId(env, cycle)`; the ids live in
  `wrangler.toml` as `PADDLE_PRICE_MONTHLY` / `PADDLE_PRICE_YEARLY`.
- Sandbox ids and live ids are different values. A sandbox `pri_…` left in a
  production config fails at checkout, not at deploy, so treat swapping them as
  part of the environment switch rather than a follow-up.

Configuration splits by secrecy, not by convenience:

| Where | What |
|---|---|
| `wrangler.toml` `[vars]` | `PADDLE_ENV`, the two `pri_…` price ids, `PADDLE_CLIENT_TOKEN` (publishable — it appears in the page source) |
| `wrangler secret put` | `PADDLE_API_KEY`, `PADDLE_WEBHOOK_SECRET` |

Never write a secret into `wrangler.toml`; that file is deployed. Never put
`DEV_MODE` there either — it belongs in `.dev.vars`, which `wrangler dev` reads
and `wrangler deploy` ignores.

`paddleConfigured(env)` is what the buy page asks before offering checkout. A
missing price or token must degrade to "checkout unavailable", never to a
charge of zero.

### Webhook verification

`verifyWebhook` in `service/src/paddle.ts` is the only way a webhook may enter
the system. Do not add a second path, and do not parse a body before it
verifies.

The method, which must not drift:

1. Read the `Paddle-Signature` header and split it on `;` into `ts=` and `h1=`.
2. Reject if either is missing.
3. Reject if `|now - ts|` exceeds `MAX_SKEW` (300s). Paddle suggests five
   seconds; that is right for an always-warm server and wrong for a Worker that
   may be cold-starting an isolate.
4. Compute `HMAC-SHA256(PADDLE_WEBHOOK_SECRET, `${ts}:${raw}`)` over the
   **raw** body text, hex-encoded.
5. Compare with `constantTimeEqual`. A webhook signature is exactly the kind of
   value an attacker gets to retry indefinitely.
6. Only then `JSON.parse`.

The timestamp is inside the MAC, so it cannot be edited to make an old webhook
look fresh. A valid signature over a stale timestamp is a replay, and both
halves have to hold.

Events handled today: `transaction.completed`, `subscription.canceled`,
`adjustment.created`. Webhooks are retried, so every handler must be
idempotent — look the licence up by `provider_ref` before creating one.

### Ask before doing anything destructive

Live Paddle entities have real customers, subscriptions and payment history
attached. Migration and fixes are **additive and code-side**. Stop and ask
first for any of these, naming the entity and the exact change:

- **Notification destinations.** Never delete or recreate one that exists.
  Recreating rotates `endpoint_secret_key` and silently breaks verification of
  every future delivery. Reuse an existing destination; only create one if none
  exists.
- **Prices and products.** Prices are immutable once used. "Fixing" a wrong
  price means creating a **new** price and pointing the config at it — never
  editing or deleting the old one, which live subscriptions still reference.
- **Customers, subscriptions, transactions**, and their mirrored rows in D1.
- **The sandbox account.** It is the reference for what live should look like.
  Do not clean it up, and do not offer to.

Refunds and cancellations are not the same operation here, and the difference
is deliberate — see `isLive` and `endLicence` in `service/src/licences.ts`. A
cancellation leaves `expires_at` alone, because someone who cancels a yearly
licence in month two has paid for ten more months and is entitled to them. A
refund cuts the term to now, because the money went back. Do not "simplify"
these into one path.

### Keeping the client and the service honest

The dollar prices appear in three places and must agree: `USD_PRICE` in
`service/src/env.ts`, `PRICE_MONTHLY` / `PRICE_YEARLY` in `src/license.rs`, and
the `price.*` translation keys on the website. The lira prices are **not** a
conversion of them; PayTR settles in lira and those numbers are set
deliberately.

The Ed25519 public key in `src/license.rs` must match the `SIGNING_KEY_PUBLIC`
secret on the deployed service. Both halves come from one run of
`service/scripts/keygen.mjs`. Rotating the key requires shipping a new binary,
so it is not a routine operation.
