-- Drops everything schema.sql creates, so a local database can be rebuilt
-- from scratch. `CREATE TABLE IF NOT EXISTS` will not alter a table that
-- already exists, so a schema change during development needs this first.
--
-- Local development only. Never run against the production database: it
-- deletes every licence, and licences cannot be reconstructed from the
-- processors -- only the payments can.
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS seats;
DROP TABLE IF EXISTS licences;
DROP TABLE IF EXISTS service_keys;
