DROP INDEX IF EXISTS idx_albums_codec;
ALTER TABLE albums DROP CONSTRAINT IF EXISTS uq_albums_provider_album_codec_part;
ALTER TABLE albums DROP CONSTRAINT IF EXISTS albums_codec_check;
ALTER TABLE albums DROP COLUMN IF EXISTS codec;
ALTER TABLE albums ADD CONSTRAINT uq_albums_provider_album_part UNIQUE (provider, album_id, part_index);

DROP INDEX IF EXISTS tracks_codec_idx;
ALTER TABLE tracks DROP CONSTRAINT IF EXISTS tracks_provider_track_id_codec_unique;
ALTER TABLE tracks DROP CONSTRAINT IF EXISTS tracks_codec_check;
ALTER TABLE tracks DROP COLUMN IF EXISTS codec;
ALTER TABLE tracks ADD CONSTRAINT tracks_provider_track_id_unique UNIQUE (provider, track_id);
