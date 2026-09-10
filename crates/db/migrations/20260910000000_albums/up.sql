CREATE TABLE albums (
    id SERIAL PRIMARY KEY,
    provider VARCHAR(32) NOT NULL,
    album_id TEXT NOT NULL,
    part_index INTEGER NOT NULL,
    total_parts INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    file_id TEXT NOT NULL,
    file_unique_id TEXT NOT NULL,
    file_size BIGINT NOT NULL,
    file_name TEXT NOT NULL,
    generation_hash VARCHAR(64) NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_albums_provider_album_part UNIQUE (provider, album_id, part_index),
    CONSTRAINT ck_albums_part_index_positive CHECK (part_index > 0),
    CONSTRAINT ck_albums_total_parts_positive CHECK (total_parts > 0),
    CONSTRAINT ck_albums_part_index_within_total CHECK (part_index <= total_parts),
    CONSTRAINT ck_albums_message_id_positive CHECK (message_id > 0),
    CONSTRAINT ck_albums_file_size_positive CHECK (file_size > 0)
);

CREATE INDEX idx_albums_album ON albums (provider, album_id);
CREATE UNIQUE INDEX idx_albums_file_unique_id ON albums (file_unique_id);
