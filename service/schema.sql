-- The licence service's whole world: who bought what, and which machines are
-- currently using it. Nothing here touches a translation.

CREATE TABLE IF NOT EXISTS licences (
  -- Goes into the token's `lic` claim, so it is safe to show and safe to log.
  id           TEXT PRIMARY KEY,
  -- SHA-256 of the key as the customer sees it. The key itself is never
  -- stored here: a leaked database must not hand anyone a working licence.
  -- The one place a plaintext key exists is `orders.licence_key`, and only
  -- for the hour it takes the buyer to read it off the success page.
  key_hash     TEXT NOT NULL UNIQUE,
  plan         TEXT NOT NULL,
  -- monthly | yearly. Display only, but it is also what a renewal extends by.
  cycle        TEXT NOT NULL DEFAULT 'monthly',
  -- NULL means unlimited, matching the client's Option<u32>. Named for what
  -- it is: a total allowance, not a rate. Only ever set on free licences.
  translation_limit INTEGER,
  -- active | cancelled | refunded. Anything but active stops the next
  -- refresh, and the entitlement lapses on its own within the token's TTL.
  status       TEXT NOT NULL DEFAULT 'active',
  seat_limit   INTEGER NOT NULL DEFAULT 3,
  -- Unix seconds at which the paid term ends. This is the authority: a token
  -- is never minted to outlive it by more than the grace period, which is
  -- what keeps a cancelled $2 monthly licence from running a further month
  -- for free. A renewal moves it forward; nothing else does.
  expires_at   INTEGER NOT NULL,
  -- The same moment as a date, for the client to display. Never enforced.
  renews_at    TEXT,
  email        TEXT,
  -- Always 'paddle'. Decides which cancel route the account page offers, and
  -- which webhook is allowed to move this row.
  provider     TEXT NOT NULL,
  -- Paddle's own id for the thing that pays: the subscription id. How a
  -- renewal or a refund finds this row.
  provider_ref TEXT,
  created_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS licences_by_provider_ref ON licences (provider, provider_ref);
CREATE INDEX IF NOT EXISTS licences_by_email ON licences (email);

CREATE TABLE IF NOT EXISTS seats (
  licence_id  TEXT NOT NULL,
  device      TEXT NOT NULL,
  os          TEXT,
  app         TEXT,
  first_seen  INTEGER NOT NULL,
  last_seen   INTEGER NOT NULL,
  PRIMARY KEY (licence_id, device)
);

CREATE INDEX IF NOT EXISTS seats_by_licence ON seats (licence_id);

-- One row per checkout attempt, which is what lets the success page show the
-- key the moment the processor confirms the payment.
--
-- Email is the fallback delivery route, not the primary one: mail is slow,
-- lands in spam, and needs a mailer configured. A page that already knows the
-- payment succeeded can simply say the key out loud.
--
-- That is also why this is the only table with a plaintext key in it, and why
-- the key is cleared as soon as `reveal_until` passes. A stolen database of
-- orders is a stolen list of live licences for at most an hour.
CREATE TABLE IF NOT EXISTS orders (
  -- Unguessable, and the only thing protecting the reveal. Bare hex, 128 bits,
  -- carried to Paddle as customData.ref and handed back on the webhook.
  ref          TEXT PRIMARY KEY,
  provider     TEXT NOT NULL,
  cycle        TEXT NOT NULL,
  email        TEXT,
  -- pending | paid | failed
  status       TEXT NOT NULL DEFAULT 'pending',
  -- Minor units (cents) and the currency actually charged, kept so the
  -- webhook can be checked against what was asked for rather than trusted.
  amount       INTEGER,
  currency     TEXT,
  licence_id   TEXT,
  licence_key  TEXT,
  reveal_until INTEGER,
  failure      TEXT,
  created_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS orders_by_reveal ON orders (reveal_until);

-- Development only. In production the signing key arrives as a Worker secret
-- and this table stays empty -- a private key in the same database as the
-- licences it signs for defeats the point of signing them.
CREATE TABLE IF NOT EXISTS service_keys (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  pkcs8       TEXT NOT NULL,
  public_hex  TEXT NOT NULL
);

-- -- The Paddle mirror ------------------------------------------------------
--
-- What Paddle believes, copied here from verified webhooks. It is a cache of
-- someone else's records, not a second source of truth: nothing writes to
-- these tables except `mirror.ts`, and every column comes off an event.
--
-- The `licences` table above is still what entitles anyone to anything. This
-- mirror exists so that the account page can answer "what is Paddle going to
-- charge me next, and where do I change my card" without a round trip, and so
-- that a customer id can be resolved server-side rather than taken from a form.
--
-- There is deliberately no foreign key from subscriptions to customers.
-- Deliveries are at-least-once and unordered, so `subscription.created` can
-- and does arrive before `customer.created`; a constraint here would turn a
-- normal ordering into a 500 and a retry storm.

CREATE TABLE IF NOT EXISTS paddle_customers (
  customer_id TEXT PRIMARY KEY,
  email       TEXT,
  status      TEXT,
  -- `occurred_at` of the event this row was last written from, in unix
  -- seconds. Webhooks arrive out of order, so an older event must not
  -- overwrite a newer one -- this is what the upserts compare against.
  event_at    INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS paddle_subscriptions (
  subscription_id         TEXT PRIMARY KEY,
  customer_id             TEXT NOT NULL,
  -- Paddle's own vocabulary, stored verbatim: active | trialing | past_due |
  -- paused | canceled. Never translated on the way in, so that `grantsAccess`
  -- is the single place that decides what any of them mean.
  status                  TEXT NOT NULL,
  price_id                TEXT,
  product_id              TEXT,
  -- A scheduled cancellation or pause is a future intention, not a current
  -- state. It is recorded so the account page can say "ends on the 3rd", and
  -- it is deliberately not consulted by `grantsAccess`.
  scheduled_change_action TEXT,
  scheduled_change_at     TEXT,
  next_billed_at          TEXT,
  event_at                INTEGER NOT NULL DEFAULT 0,
  created_at              INTEGER NOT NULL,
  updated_at              INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS paddle_subscriptions_by_customer
  ON paddle_subscriptions (customer_id);
