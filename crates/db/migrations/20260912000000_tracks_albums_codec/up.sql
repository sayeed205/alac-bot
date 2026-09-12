ALTER TABLE tracks ADD COLUMN codec VARCHAR(16) NOT NULL DEFAULT 'alac';
ALTER TABLE tracks ADD CONSTRAINT tracks_codec_check CHECK (codec IN ('alac', 'ec-3', 'aac', 'flac'));
ALTER TABLE tracks DROP CONSTRAINT tracks_provider_track_id_unique;
ALTER TABLE tracks ADD CONSTRAINT tracks_provider_track_id_codec_unique UNIQUE (provider, track_id, codec);
CREATE INDEX tracks_codec_idx ON tracks (codec);

ALTER TABLE albums ADD COLUMN codec VARCHAR(16) NOT NULL DEFAULT 'alac';
ALTER TABLE albums ADD CONSTRAINT albums_codec_check CHECK (codec IN ('alac', 'ec-3', 'aac', 'flac'));
ALTER TABLE albums DROP CONSTRAINT uq_albums_provider_album_part;
ALTER TABLE albums ADD CONSTRAINT uq_albums_provider_album_codec_part UNIQUE (provider, album_id, codec, part_index);
CREATE INDEX idx_albums_codec ON albums (codec);
