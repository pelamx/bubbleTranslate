-- The licence service's whole world: who bought what, and which machines are
-- currently using it. Nothing here touches a translation.

CREATE TABLE IF NOT EXISTS licences (
  -- Goes into the token's `lic` claim, so it is safe to show and safe to log.
  id           TEXT PRIMARY KEY,
  -- SHA-256 of the key as the customer sees it. The key itself is never
  -- stored: a leaked database must not hand anyone a working licence.
  key_hash     TEXT NOT NULL UNIQUE,
  plan         TEXT NOT NULL,
  -- NULL means unlimited, matching the client's Option<u32>.
  daily_limit  INTEGER,
  -- active | refunded | cancelled. Anything but active stops the next
  -- refresh, and the entitlement lapses on its own within the token's TTL.
  status       TEXT NOT NULL DEFAULT 'active',
  seat_limit   INTEGER NOT NULL DEFAULT 3,
  -- Displayed by the client, never enforced by it.
  renews_at    TEXT,
  email        TEXT,
  order_ref    TEXT,
  created_at   INTEGER NOT NULL
);

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

-- Development only. In production the signing key arrives as a Worker secret
-- and this table stays empty -- a private key in the same database as the
-- licences it signs for defeats the point of signing them.
CREATE TABLE IF NOT EXISTS service_keys (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  pkcs8       TEXT NOT NULL,
  public_hex  TEXT NOT NULL
);
