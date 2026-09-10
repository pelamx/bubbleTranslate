# The licence service

Three endpoints the app calls, four pages a buyer's browser calls, and two
webhooks the payment processors call. It is never on the path of a translation
and never sees one — all it decides is whether a licence key still entitles a
machine to have its counter lifted, and it says so in a token the app then
checks on its own for up to thirty days.

If this service is down, every install degrades to the free allowance rather than
to a broken app. That is the reason the client was built to work before this
existed, and the reason it should stay that way.

## What is sold

**bubbleTranslate Pro** — unlimited translations, three machines.

| | Everywhere |
|---|---|
| Processor | Paddle |
| Monthly | $2 |
| Yearly | $20 |
| Renews itself | yes |
| Tax and invoicing | Paddle's, as merchant of record |

Paddle prices the transaction in the buyer's own currency and adds the local
tax, so $2/$20 are the figures the product quotes rather than what every card
is debited.

The free tier is ten translations a day, counted at the user's local midnight.
The number travels in the token as `lim` and falls back to the compiled-in ten
when there is no token. See `src/quota.rs` in the client.

### Where the prices live

The dollar figures are the product's, and they appear in three places that
must agree: `USD_PRICE` in `src/env.ts`, `PRICE_MONTHLY` / `PRICE_YEARLY` in
the client's `src/license.rs`, and the `price.*` keys on the website. The
amount actually charged is Paddle's, set on the price ids in the catalogue —
`paddleConfigured` degrades `/buy` to "checkout unavailable" if a price id or
the client token is missing, and never to a charge of zero.

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
tomorrow's tokens. `DEV_MODE` is what opens `/v1/pubkey` and `/v1/dev/issue`.
It lives in `.dev.vars`, which `wrangler dev` reads and `wrangler deploy`
ignores, so it cannot reach production by accident:

```
DEV_MODE = "1"
PUBLIC_BASE_URL = "http://localhost:8787"
```

Processor credentials for local work go in the same file, which is gitignored.

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
| `GET /buy` | The pricing page and the Paddle.js overlay |
| `POST /checkout/paddle` | Mints an order ref for the Paddle.js overlay |
| `GET /welcome?ref=` | Polls until the payment lands, then shows the key |
| `GET /v1/order/:ref` | What that page polls |
| `GET|POST /account` | Licence status, and cancellation where it applies |

`/buy` passes the edge's country to Paddle only as a hint for its price
preview, and never a sentinel: Cloudflare's `XX` (cannot place) and `T1` (Tor)
are dropped, and Paddle geolocates the IP itself instead.

### Languages

Every page speaks English, Turkish and Spanish, switched from the EN / TR / ES
links at the top right. English is the default for everyone — the language is
never guessed from the IP address. The choice rides on `?lang=`, on a hidden
field in each form, and on a `lang` cookie so the return trip from Paddle comes
back in the same language. The strings
live in `src/i18n.ts`, one object per language, typed against the English one
so a string added without its two translations fails to compile.

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
| `/webhooks/paddle` | `Paddle-Signature: ts=…;h1=…` over `ts:body` | `{ok}` or 401 |

Paddle retries on any non-2xx, so fulfilment is idempotent: an order already
marked paid issues nothing further, and a licence is looked up by
`provider_ref` before another is created.

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

Refunds and cancellations should normally be done in Paddle, whose webhook
updates this automatically. The Refund button only marks the licence;
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
3. Check `PUBLIC_BASE_URL` in `wrangler.toml` is the real hostname. The
   success URL Paddle returns the buyer to is built from it. `DEV_MODE` is
   never in that file; see above.
4. `wrangler secret put ADMIN_PASSWORD`, or leave it unset and have no panel.
5. **Paddle**: create the two prices in the catalogue, put their `pri_…` ids
   and the client token in `wrangler.toml`, set `PADDLE_ENV = "production"`,
   and add **one** notification destination pointing at `/webhooks/paddle`,
   subscribed to `transaction.completed`, `subscription.created`,
   `subscription.updated`, `subscription.canceled` and `adjustment.created`.
   `PADDLE_WEBHOOK_SECRET` must be that destination's signing secret; never
   delete and recreate a destination, because that rotates the secret.
   ```sh
   wrangler secret put PADDLE_API_KEY
   wrangler secret put PADDLE_WEBHOOK_SECRET
   ```
6. Attach `api.bubbletranslate.app` as a custom domain, and only then cut a
   release binary. `LICENSE_API` and `BUY_URL` are compiled in: ship against a
   temporary hostname and every install keeps calling it forever.

Buy one with a real card before announcing it. The paths worth walking end to
end are a purchase, a renewal, a refund, and a cancellation — the last three
because they are the ones that only fail later.
