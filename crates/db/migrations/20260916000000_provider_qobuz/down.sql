ALTER TABLE tracks DROP CONSTRAINT IF EXISTS tracks_provider_check;
ALTER TABLE tracks
    ADD CONSTRAINT tracks_provider_check CHECK (provider IN ('apple')) NOT VALID;

ALTER TABLE requests DROP CONSTRAINT IF EXISTS requests_provider_check;
ALTER TABLE requests
    ADD CONSTRAINT requests_provider_check CHECK (provider IN ('apple')) NOT VALID;

ALTER TABLE albums DROP CONSTRAINT IF EXISTS albums_provider_check;
ALTER TABLE albums
    ADD CONSTRAINT albums_provider_check CHECK (provider IN ('apple')) NOT VALID;
