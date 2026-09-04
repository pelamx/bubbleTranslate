# The licence service

Three endpoints the app calls, four pages a buyer's browser calls, and two
webhooks the payment processors call. It is never on the path of a translation
and never sees one — all it decides is whether a licence key still entitles a
machine to have its counter lifted, and it says so in a token the app then
checks on its own for up to thirty days.

If this service is down, every install degrades to the free trial rather than
to a broken app. That is the reason the client was built to work before this
existed, and the reason it should stay that way.

## What is sold

**bubbleTranslate Pro** — unlimited translations, three machines.

| | Turkey | Everywhere else |
|---|---|---|
| Processor | PayTR | Paddle |
| Monthly | set in `wrangler.toml` | $2 |
| Yearly | set in `wrangler.toml` | $20 |
| Renews itself | no — fixed term | yes |
| Tax and invoicing | yours | Paddle's, as merchant of record |

The free tier is ten translations, once, for the life of the install — not per
day. It does not reset, which is why the wording throughout the app says
"trial" rather than "allowance". See `src/quota.rs` in the client.

### The two prices that are not in this repo

`PAYTR_PRICE_MONTHLY_KURUS` and `PAYTR_PRICE_YEARLY_KURUS` in `wrangler.toml`
start empty, and until they are set `/buy` tells Turkish visitors that checkout
is unavailable and offers them the dollar page instead. It never charges zero.

They are deliberately not a conversion of $2 and $20. PayTR settles in lira,
and a price that follows the exchange rate is one no customer can budget for
and no accountant can reconcile. Pick a number and revisit it when you mean to.

### Why Turkish licences do not auto-renew

PayTR's recurring product has to be enabled per merchant account and is not
available to every seller, so the default here is a term the buyer chooses and
re-buys. `extendTerm` already handles a renewal arriving for an existing
licence, so switching this on later is a webhook change rather than a redesign.

The consequence is visible to customers and worth being deliberate about: a
Turkish buyer gets a licence that ends on a date, and the account page says so
instead of offering a cancel button.

## Running it locally

No domain, no Cloudflare account, no payment processor.

```sh
npm install
npm run schema          # creates the local D1 tables
npm run dev             # http://localhost:8787
```

`npm run schema:reset` drops and rebuilds them, which is what a schema change
during development needs — `CREATE TABLE IF NOT EXISTS` will not alter a table
that already exists. Local only; it deletes every licence.

Then point the app at it. Both overrides are debug-only — a release build
ignores them, since a settable public key would be a licence bypass anyone
could export:

```sh
curl -s localhost:8787/v1/pubkey     # {"public_key_hex":"..."}

BUBBLETRANSLATE_LICENSE_API=http://localhost:8787 \
BUBBLETRANSLATE_LICENSE_PUBKEY=<that hex> \
cargo run
```

Issue yourself a key without a processor, then paste it into the Account tab:

```sh
curl -s -X POST localhost:8787/v1/dev/issue -d '{"cycle":"yearly"}'
# {"id":"lc_...","key":"BT-M3JDS-X5GTW-T42UC","cycle":"yearly","expires_at":...}
```

`term_seconds` shortens the paid term, which is how the expiry and grace
behaviour is exercised without waiting a year.

In development the service generates its own Ed25519 key on first use and
keeps it in D1, so the public key you started the app with still verifies
tomorrow's tokens. `DEV_MODE` in `wrangler.toml` is what opens `/v1/pubkey`
and `/v1/dev/issue`; it must never be set in production.

Processor credentials for local work go in `.dev.vars`, which is gitignored.

## The contract

### Routes the app calls

All `POST`, all JSON. A non-200 returns `{"error": "..."}`, and **that string
is shown to the user verbatim** — write sentences addressed to a person.

| Route | In | Out |
|---|---|---|
| `/v1/activate` | `{key, device, os, app}` | `{token, renews}` |
| `/v1/refresh` | `{token, device}` | `{token, renews}` |
| `/v1/deactivate` | `{token, device}` | `{ok}` — reply ignored by the client |

The token is `b64url(claims).b64url(signature)`, where the signature covers the
**ASCII bytes of the encoded payload**, not the raw JSON. The client verifies
before it parses, so nothing inside an unsigned blob gets to choose which
parser runs on it. Claims are `{lic, plan, cyc?, lim, dev, iat, exp}`, with
`lim: null` meaning unlimited and `cyc` omitted rather than null when absent.
The client rejects `exp <= now`, `iat` more than a day in the future, and any
`dev` that is not its own machine.

Seats default to three per licence. Re-activating a machine that already holds
one is not a new seat; *Remove from this device* in the Account tab frees it.

### Routes a browser calls

| Route | What it does |
|---|---|
| `GET /buy` | Country-routed pricing. `?country=TR` overrides the guess |
| `POST /checkout/paytr` | Opens a PayTR session and serves its iframe |
| `POST /checkout/paddle` | Mints an order ref for the Paddle.js overlay |
| `GET /done?ref=` | Polls until the payment lands, then shows the key |
| `GET /v1/order/:ref` | What that page polls |
| `GET|POST /account` | Licence status, and cancellation where it applies |

Geolocation is a guess, so both variants of `/buy` link to the other. A Turkish
customer on a VPN, or someone abroad who wants to pay in lira, must not be
stuck with the wrong processor because Cloudflare read an IP a certain way.

### How the key reaches the buyer

The success page shows it, and email is the fallback rather than the route.
Mail is slow, lands in spam, and needs a mailer configured; a page that already
knows the payment succeeded can simply say the key out loud.

That is why `orders` is the one table with a plaintext key in it, and why the
key is cleared once `reveal_until` passes — an hour. Set `RESEND_API_KEY` and
`MAIL_FROM` to also send it; without them the webhook logs that it could not.

### Webhooks

| Route | Signature | Reply |
|---|---|---|
| `/webhooks/paytr` | `hash` field, HMAC-SHA256, base64 | the literal `OK`, always |
| `/webhooks/paddle` | `Paddle-Signature: ts=…;h1=…` over `ts:body` | `{ok}` or 401 |

PayTR must be answered `OK` and nothing else or it keeps retrying and
eventually flags the merchant account — including when the signature does not
verify, because there is nothing it could usefully retry.

Both processors retry, so fulfilment is idempotent: an order already marked
paid issues nothing further. PayTR callbacks are also checked against the
amount the order was created for, so a success for less than the plan costs
stops there rather than becoming a licence.

Paddle events handled: `transaction.completed` (a first payment when it
carries our `custom_data.ref`, a renewal when it names a subscription we
already know), `subscription.canceled`, and `adjustment.created` with
`action: refund`. Anything else is acknowledged and ignored.

### How a licence ends

`expires_at` is the authority, not `status`. A token is never minted to outlive
the paid term by more than three days' grace, which is what stops someone who
cancels a $2 monthly licence the day after paying from keeping Pro for a
further month.

- **Cancelled** leaves the term alone. It has been paid for, and the subscriber
  keeps Pro until it ends.
- **Refunded** cuts the term to now. The money went back.
- **Expired** is neither — the term simply ran out.

All three lapse on their own within the token's remaining life. There is no
revocation list to poll and no moment where a working app has to ask
permission to keep working.

## The admin panel

`/admin`, behind HTTP Basic against the `ADMIN_PASSWORD` secret. Leave that
secret unset and the route answers 404 rather than 401 — an unconfigured
deployment should look like a service with no panel, not like one with a panel
and a lock to pick.

```sh
wrangler secret put ADMIN_PASSWORD
```

It shows live subscriber counts split by plan and processor, what is ending in
the next seven days, and any checkout that failed in the last week — a run of
those is a broken price, a wrong credential or a processor outage, and nothing
else reports them.

Search takes whatever support arrives with: an email address, a licence key
(hashed before lookup, since that is the only form stored), an `lc_…` id, or a
processor subscription ref.

Four actions per licence, which are the four things support actually has to do:

| Action | What it does |
|---|---|
| **+30d** | Extends the term, from the current expiry or from now, whichever is later |
| **Free seats** | Drops every device, so someone who reinstalled can activate again |
| **New key** | Issues a replacement key and kills the old one |
| **Refund** | Marks the licence refunded, ending Pro immediately |

**New key** is the answer to "I lost my key", which otherwise has no answer:
`licences` holds only the hash and the plaintext copy in `orders` is swept
within the hour. It replaces rather than duplicates — the same licence keeps
its id, term, subscription and seats, and **machines already activated are
undisturbed**, because `refresh` finds the licence by the token's `lic` claim
rather than by the key. Only the old key stops working, which makes this the
right tool for a key that leaked as well as one that was lost.

Refunds and cancellations should normally be done in PayTR or Paddle, whose
webhooks update this automatically. The Refund button only marks the licence;
it moves no money. Doing it in both places is how the two come to disagree.

Every state-changing route checks the request's `Origin`. A browser holding
Basic credentials will attach them to any request it is talked into making,
including a form on someone else's page.

## Going to production

1. `wrangler d1 create bubbletranslate-licence`, put the real `database_id`
   in `wrangler.toml`, then `npm run schema:remote`.
2. Generate the production keypair **once**, off this service. Put the
   private half in a secret and the public half in the client:
   ```sh
   wrangler secret put SIGNING_KEY_PKCS8    # base64url PKCS#8
   wrangler secret put SIGNING_KEY_PUBLIC   # 32 bytes as hex
   ```
   The same hex goes into `PUBLIC_KEY_HEX` in `src/license.rs`. If that
   private key ever leaks, anyone can mint Pro licences and the only fix is
   shipping a new binary — treat it accordingly.
3. Remove `DEV_MODE` from `wrangler.toml`, and set `PUBLIC_BASE_URL` to the
   real hostname. The return URLs PayTR is given are built from it.
4. **PayTR**: set the merchant secrets, set the two lira prices, set
   `PAYTR_TEST_MODE = "0"`, and point the notification URL in the PayTR panel
   at `/webhooks/paytr`.
   ```sh
   wrangler secret put PAYTR_MERCHANT_ID
   wrangler secret put PAYTR_MERCHANT_KEY
   wrangler secret put PAYTR_MERCHANT_SALT
   ```
5. `wrangler secret put ADMIN_PASSWORD`, or leave it unset and have no panel.
6. **Paddle**: create the two prices in the catalogue, put their `pri_…` ids
   and the client token in `wrangler.toml`, set `PADDLE_ENV = "production"`,
   and add a notification destination pointing at `/webhooks/paddle`.
   ```sh
   wrangler secret put PADDLE_API_KEY
   wrangler secret put PADDLE_WEBHOOK_SECRET
   ```
7. Attach `api.bubbletranslate.app` as a custom domain, and only then cut a
   release binary. `LICENSE_API` and `BUY_URL` are compiled in: ship against a
   temporary hostname and every install keeps calling it forever.

Buy one of each with a real card before announcing it. The paths worth walking
end to end are a Turkish purchase, an international one, a refund, and a
cancellation — the last two because they are the ones that only fail later.
