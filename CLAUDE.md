# bubbleTranslate

A bubble translator for macOS, Windows and Linux. Select text in any
application and a small bubble appears at the cursor with the translation.

- `src/` — the Rust client (egui bubble, selection monitor, translation chain,
  licence check, daily allowance). `src/platform/` holds the three
  implementations of "what is selected, when did it finish, where is the
  pointer"; everything above that line is the same code everywhere.
- `service/` — the licence service, a Cloudflare Worker on D1. It sells Pro,
  issues licence keys and signs entitlement tokens. It is **never** on the path
  of a translation.
- The website lives in a separate repository, `pelamx/bubbletranslate.app`.

## Every change is written down for the people using it

`CHANGELOG.md` at the root is the record, and keeping it current is part of
making a change rather than a step to raise afterwards. **Do not ask whether to
add an entry — add it.** The same rule holds on every machine: this repository
is the same on the Mac, on Windows and on Linux, and a release from any of them
is expected to arrive with its entry already written.

What earns an entry: anything someone running the app could notice. A fix, a
new behaviour, a changed default, a different download, a rename. What does
not: refactoring, tests, comments, the website's own styling, and anything
that leaves the app behaving exactly as before.

How to write one, which is the part that matters:

- Write for the person using the app, not for the person who wrote the code.
  Name the thing they saw going wrong, not the function that was wrong.
- Say why it was worth changing. An entry that only says *what* changed makes
  a reader work out whether it affects them; one sentence of *why* answers it.
- Mark a change that only affects one system — `(Windows)`, `(macOS)`,
  `(Linux)` — because the three are released separately.
- No commit hashes, no file paths, no internal names. If a sentence cannot be
  written without one, it is probably not an entry.

New entries go under `## Unreleased`. Releasing turns that heading into the
version and the date; the release scripts refuse to build a version that has no
section of its own, the same way they warn about a version that was never
bumped. That section is also the body of the GitHub release, pasted as it is —
the release page is where someone who just saw "a new version is available"
ends up, so it is where the explanation has to be.

## Writing the name

The product is **bubbleTranslate** — lowercase `b`, capital `T`, one word.
Never `BubbleTranslate`, never `Bubble Translate`, never `bubbletranslate` as a
word. It is written that way everywhere a person can read it: the interface,
the website in all three languages, page titles, meta tags, alt text, release
notes, commit messages and prose in this repository.

Two things are not the name and stay as they are: the domain and URLs
(`bubbletranslate.app`, all-lowercase because hostnames are), and identifiers a
language or platform has its own convention for — `BubbleTranslateStatusTarget`
in `src/platform/macos/shell.rs` is an Objective-C class name, which nobody
reads and which is capitalised the way Objective-C classes are.

A sentence that begins with the name still begins with a lowercase `b`. That
looks wrong for a moment and is correct: it is a name, not a word.

Run `cargo test` before proposing a client change. The suite includes an
integration test that drives the real binary, so a change to startup ordering
is caught rather than assumed safe.

## Releases

**Run the build before publishing it.** `cargo test` passing is not a release
test: the suite never opens a window, so it cannot see a bubble that comes up
black, a translation that never arrives, or a tray icon that is not there. The
binary that is about to be uploaded is the one to start — select text in
another application, watch the bubble appear, and read the translation in it.
A release that was only compiled has not been tested, and saying "the tests
pass" about one is a way of not saying that.

This is not optional and it is not a nicety to raise afterwards: it is 0.2.7
for Windows, which shipped with a black bubble and no translation because the
build was uploaded without ever being launched.

**The Windows download is published as a zip, not as a bare `.exe`.** This is
settled; it does not need asking again. A browser handed an unsigned `.exe`
says it "isn't commonly downloaded" and throws it away unless the user digs it
back out of the warning, and the same bytes inside a zip arrive without it — at
7 MB rather than 17. `release.ps1` zips after signing, so what goes in the
archive is the signed executable. `latest.json` and every download link point
at `bubbleTranslate-windows-x64.zip`; the bare `.exe` is uploaded beside it for
anyone who wants it, and nothing links to it as the primary download.

**The downloads live in `pelamx/downloads`, not here.** This repository holds
the source; that one is public and is where every release asset and
`latest.json` are published. An installed copy reads `latest.json` from it on
startup, so anything that takes it offline or renames it stops the update
notice for every copy already out there. Keeping the two apart is what lets
this repository be made private again without taking the downloads with it.

They lived in `bubbleTranslate/downloads` until 0.3.6, under a second GitHub
account that only that account could publish to. That repository is frozen now
and read-only to us; its assets stay where they are because copies of 0.3.5 and
older ask it for `latest.json` and will go on doing so for as long as they are
running. Point nothing new at it.

Copies of 0.2.7 and older read the manifest from a third address —
`latest.json` at the root of **this** repository. That file is a copy kept by
`.github/workflows/mirror-manifest.yml`, which follows the downloads
repository every 15 minutes. Never edit it by hand and never delete it: it
only works while this repository is public, and without it those copies are
never told about an update.

One script per platform, each rewriting only its own line of `latest.json` —
fetched from the downloads repository, patched and put back through the API,
so a release on one machine leaves the other two platforms alone:
`release.sh` (macOS), `release.ps1` (Windows), `release-linux.sh` (Linux);
the shared half is `scripts/publish-manifest.sh`. Each warns when
`Cargo.toml` was not bumped, because a release that reuses the published
version tells nobody. The version lives in `Cargo.toml` and in that
`latest.json` and nowhere else: the links in README.md and on the website are
`/releases/latest/download/…`, which GitHub resolves to the current release, so
publishing one is what moves them.

Every release machine therefore needs `gh` logged in to the `bubbleTranslate`
account, which owns the downloads repository.

## Paddle

Paddle is how everyone pays — it is the only processor. Paddle is the
**merchant of record**: it sells the licence to the customer and we sell it to
Paddle, which is why VAT and invoicing are Paddle's problem rather than ours in
every country at once.

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

Events that move a licence today: `transaction.completed` (buys or renews),
`subscription.canceled` (ends the term's recurrence) and `adjustment.created`
(a refund cuts the term short). Alongside them `subscription.created`,
`subscription.updated`, `customer.created` and `customer.updated` are handled
too, but only to keep the D1 mirror current — they entitle nobody on their own.
Webhooks are retried, so every handler must be idempotent — look the licence up
by `provider_ref` before creating one.

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
the `price.*` translation keys on the website. Paddle prices the transaction
in the buyer's own currency and adds tax, so these dollar figures are what the
product *says*, not what every card is debited.

The Ed25519 public key in `src/license.rs` must match the `SIGNING_KEY_PUBLIC`
secret on the deployed service. Both halves come from one run of
`service/scripts/keygen.mjs`. Rotating the key requires shipping a new binary,
so it is not a routine operation.
