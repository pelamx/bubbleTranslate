# The licence service

Three endpoints the app calls, and one the payment processor calls. It is
never on the path of a translation and never sees one — all it decides is
whether a licence key still entitles a machine to have its counter lifted,
and it says so in a token the app then checks on its own for thirty days.

If this service is down, every install degrades to the free tier rather than
to a broken app. That is the reason the client was built to work before this
existed, and the reason it should stay that way.

## Running it locally

No domain, no Cloudflare account, no payment processor.

```sh
npm install
npm run schema          # creates the local D1 tables
npm run dev             # http://localhost:8787
```

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
curl -s -X POST localhost:8787/v1/dev/issue -d '{"plan":"pro"}'
# {"id":"lc_...","key":"BT-M3JDS-X5GTW-T42UC"}
```

In development the service generates its own Ed25519 key on first use and
keeps it in D1, so the public key you started the app with still verifies
tomorrow's tokens. `DEV_MODE` in `wrangler.toml` is what opens `/v1/pubkey`
and `/v1/dev/issue`; it must never be set in production.

## The contract

All `POST`, all JSON. A non-200 returns `{"error": "..."}`, and **that string
is shown to the user verbatim** — write sentences addressed to a person.

| Route | In | Out |
|---|---|---|
| `/v1/activate` | `{key, device, os, app}` | `{token, renews}` |
| `/v1/refresh` | `{token, device}` | `{token, renews}` |
| `/v1/deactivate` | `{token, device}` | `{ok}` — reply ignored by the client |
| `/webhooks/lemonsqueezy` | processor event | `{ok}` |

The token is `b64url(claims).b64url(signature)`, where the signature covers
the **ASCII bytes of the encoded payload**, not the raw JSON. The client
verifies before it parses, so nothing inside an unsigned blob gets to choose
which parser runs on it. Claims are `{lic, plan, lim, dev, iat, exp}`, with
`lim: null` meaning unlimited. The client rejects `exp <= now`, `iat` more
than a day in the future, and any `dev` that is not its own machine.

Seats default to three per licence. Re-activating a machine that already
holds one is not a new seat; Deactivate in the Account tab frees it.

Revocation is by omission. A refund or cancellation flips `status`, which
stops the next refresh, and the entitlement lapses on its own within the
token's thirty days. There is no revocation list to poll and no moment where
a working app has to ask permission to keep working.

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
3. Remove `DEV_MODE` from `wrangler.toml`.
4. Set `LEMONSQUEEZY_WEBHOOK_SECRET` and point the processor's webhook at
   `/webhooks/lemonsqueezy`. **Emailing the key to the customer is still
   yours to wire up** — today the webhook creates the licence and logs the
   key against the order.
5. Attach `api.bubbletranslate.app` as a custom domain, and only then cut a
   release binary. `LICENSE_API` is compiled in: ship against a temporary
   hostname and every install keeps calling it forever.
