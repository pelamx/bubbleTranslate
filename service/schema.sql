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

-- One row per install that has sent the daily usage ping (see
-- `license::ping` in the client). `install` is a salted hash that cannot be
-- joined to `seats.device`. Holds no text, no languages and no licence.
--
-- `country` is the two-letter code Cloudflare works out from the request's
-- address; the address itself is never stored. The `capped` columns record
-- when the free allowance ran out: the first time, and on how many different
-- days. The four were added on 2026-09-25 to a live table, with
--   ALTER TABLE installs ADD COLUMN country TEXT;
--   ALTER TABLE installs ADD COLUMN first_capped INTEGER;
--   ALTER TABLE installs ADD COLUMN last_capped INTEGER;
--   ALTER TABLE installs ADD COLUMN capped_days INTEGER NOT NULL DEFAULT 0;
--
-- `source` is where the install most likely came from: the `src` of the
-- website download click it was matched to on its first ping (see
-- `download_clicks`), or 'unmatched' when no click fits. NULL for installs
-- first seen before the matching existed. Added on 2026-10-02 with
--   ALTER TABLE installs ADD COLUMN source TEXT;
CREATE TABLE IF NOT EXISTS installs (
  install      TEXT PRIMARY KEY,
  os           TEXT,
  app          TEXT,
  plan         TEXT,
  first_seen   INTEGER NOT NULL,
  last_seen    INTEGER NOT NULL,
  country      TEXT,
  first_capped INTEGER,
  last_capped  INTEGER,
  capped_days  INTEGER NOT NULL DEFAULT 0,
  source       TEXT
);

CREATE INDEX IF NOT EXISTS installs_by_last_seen ON installs (last_seen);

-- Every time an install's daily ping arrives with a different version from
-- the one it last reported: that copy was updated (or, rarely, went back).
-- Same salted install id as `installs`, nothing else about the person. Before
-- this table only the current version was kept, so an update could not be told
-- from a new install on the new version. Kept for 13 months,
-- like the install itself. Added on 2026-10-05 to a live database with this
-- statement and the index below.
CREATE TABLE IF NOT EXISTS updates (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  install  TEXT NOT NULL,
  os       TEXT,
  from_app TEXT NOT NULL,
  to_app   TEXT NOT NULL,
  at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS updates_by_at ON updates (at);

-- How each translation backend fared, per day, added up across every install.
--
-- Totals only: no install id, so this says "Google failed 400 times yesterday"
-- and can never say who it failed for. It exists because the leading backend is
-- an undocumented endpoint that may start refusing at any time, and without
-- this the day it breaks for everyone looks like a quiet day.
--
-- An install reports the running total for its own day once per ping, so the
-- figures are a close sum rather than an exact one -- a copy that pings either
-- side of midnight contributes twice. Good enough to see a backend fall over,
-- which is all it is for.
CREATE TABLE IF NOT EXISTS provider_health (
  day       TEXT    NOT NULL,
  provider  TEXT    NOT NULL,
  ok        INTEGER NOT NULL DEFAULT 0,
  failed    INTEGER NOT NULL DEFAULT 0,
  reports   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, provider)
);

-- Paddle transactions already acted on, one row each. Webhooks are delivered
-- at least once, and a repeated `transaction.completed` for a subscription the
-- licence already knows looks exactly like a renewal: without this, every
-- redelivery would add another free month or year. Claimed before the work is
-- done and released if it fails, so a retry after an error still goes through.
CREATE TABLE IF NOT EXISTS paddle_transactions (
  transaction_id TEXT PRIMARY KEY,
  at             INTEGER NOT NULL
);

-- Machines added to a licence, kept so the number of *new* machines in a month
-- can be capped. Seats alone cannot do it: a seat that is removed frees its
-- slot, but the token that machine already holds works offline for its whole
-- lifetime, so activating and removing in turn would put one key on any number
-- of machines. Emptied along with the seats when the operator frees them.
CREATE TABLE IF NOT EXISTS activations (
  licence_id  TEXT NOT NULL,
  device      TEXT NOT NULL,
  at          INTEGER NOT NULL,
  PRIMARY KEY (licence_id, device)
);

-- Installs the operator marked as their own from the admin panel. Left out of
-- every admin count, so the panel shows real users only.
CREATE TABLE IF NOT EXISTS ignored_installs (
  install     TEXT PRIMARY KEY,
  created_at  INTEGER NOT NULL
);

-- Licences the operator marked as their own -- a test purchase, most of all.
-- Still listed and still working, but never counted as a customer or a sale.
CREATE TABLE IF NOT EXISTS ignored_licences (
  licence_id  TEXT PRIMARY KEY,
  created_at  INTEGER NOT NULL
);

-- What the operator did from the admin panel, one row per action. Answers
-- "why does this licence have 5 devices?" months later. Holds the licence id
-- and a one-line description of the change, never a key.
CREATE TABLE IF NOT EXISTS admin_log (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  at          INTEGER NOT NULL,
  action      TEXT NOT NULL,
  licence_id  TEXT,
  detail      TEXT
);

CREATE INDEX IF NOT EXISTS admin_log_by_licence ON admin_log (licence_id);

-- Every delivery to the Paddle webhook and what became of it, so a payment
-- that was refused or ignored shows up on the panel rather than only in a
-- log nobody reads. The event type, Paddle's ids and the outcome -- never the
-- payload, which carries the customer's details. Kept for 90 days.
CREATE TABLE IF NOT EXISTS webhook_events (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  at          INTEGER NOT NULL,
  event_type  TEXT,
  event_id    TEXT,
  entity_id   TEXT,
  -- handled | ignored | refused | bad signature | error
  outcome     TEXT NOT NULL,
  detail      TEXT
);

CREATE INDEX IF NOT EXISTS webhook_events_by_at ON webhook_events (at);

-- Every time the buy page is opened, and from where: the app's bubble when
-- the free allowance runs out, the app's window, or anywhere else. Next to
-- the orders table this is the funnel -- how many looked, how many started
-- paying, how many finished. No address and no install id, only the country
-- Cloudflare already worked out. Kept for 90 days.
CREATE TABLE IF NOT EXISTS buy_visits (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       INTEGER NOT NULL,
  src      TEXT NOT NULL,
  lang     TEXT NOT NULL,
  country  TEXT
);

CREATE INDEX IF NOT EXISTS buy_visits_by_at ON buy_visits (at);

-- Every press of a download button on the website, and where the visitor came
-- from: the `utm_source` / `ref` of the link that brought them, or the site
-- that sent them, or 'direct'. No address and no install id at the time of the
-- click -- only the country Cloudflare already worked out.
--
-- A new install's first ping claims the latest unclaimed click with the same
-- OS and country from the three days before it, and writes that click's `src`
-- into `installs.source`. It is a best guess, not a join: two people in one
-- country downloading for one OS on the same day can swap sources. Kept for
-- 90 days, which is as long as a click can be worth matching and longer.
CREATE TABLE IF NOT EXISTS download_clicks (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       INTEGER NOT NULL,
  os       TEXT NOT NULL,
  src      TEXT NOT NULL,
  country  TEXT,
  install  TEXT
);

CREATE INDEX IF NOT EXISTS download_clicks_by_at ON download_clicks (at);
