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
  -- paytr | paddle. Decides which cancel route the account page offers, and
  -- which webhook is allowed to move this row.
  provider     TEXT NOT NULL,
  -- The processor's own id for the thing that pays: a Paddle subscription id,
  -- or a PayTR merchant_oid. How a renewal or a refund finds this row.
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
  -- Unguessable, and the only thing protecting the reveal. For PayTR this is
  -- also the merchant_oid, which is why it is bare hex: PayTR rejects an oid
  -- containing anything but letters and digits.
  ref          TEXT PRIMARY KEY,
  provider     TEXT NOT NULL,
  cycle        TEXT NOT NULL,
  email        TEXT,
  -- pending | paid | failed
  status       TEXT NOT NULL DEFAULT 'pending',
  -- Minor units (kuruş, cents) and the currency actually charged, kept so the
  -- processor's callback can be checked against what was asked for rather
  -- than trusted.
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
