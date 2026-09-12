-- The initial schema accepted providers that are no longer supported. Replace
-- those checks for both fresh databases and databases created from that schema.
-- NOT VALID keeps a migration from failing on rows written by the old schema,
-- while PostgreSQL still checks every new or updated row.
ALTER TABLE tracks DROP CONSTRAINT IF EXISTS tracks_provider_check;
ALTER TABLE tracks
    ADD CONSTRAINT tracks_provider_check CHECK (provider IN ('apple')) NOT VALID;

ALTER TABLE requests DROP CONSTRAINT IF EXISTS requests_provider_check;
ALTER TABLE requests
    ADD CONSTRAINT requests_provider_check CHECK (provider IN ('apple')) NOT VALID;

-- Albums were added without a provider check in the historical migration, but
-- a partially upgraded database may already have one.
ALTER TABLE albums DROP CONSTRAINT IF EXISTS albums_provider_check;
ALTER TABLE albums
    ADD CONSTRAINT albums_provider_check CHECK (provider IN ('apple')) NOT VALID;
