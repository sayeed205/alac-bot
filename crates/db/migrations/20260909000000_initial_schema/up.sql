CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE users (
    telegram_id bigint PRIMARY KEY,
    name text,
    created_at timestamptz DEFAULT now() NOT NULL
);

CREATE TABLE tracks (
    id serial PRIMARY KEY,
    provider varchar NOT NULL,
    track_id text NOT NULL,
    message_id integer NOT NULL,
    file_id text NOT NULL,
    file_unique_id text NOT NULL,
    title text NOT NULL,
    artist text NOT NULL,
    album text NOT NULL,
    duration integer NOT NULL,
    bit_depth integer NOT NULL,
    sample_rate integer NOT NULL,
    genre text NOT NULL,
    release_date text NOT NULL,
    track_number integer NOT NULL,
    track_count integer NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT tracks_provider_check CHECK (provider IN ('apple', 'spotify', 'amazon')),
    CONSTRAINT tracks_provider_track_id_unique UNIQUE (provider, track_id)
);
CREATE INDEX tracks_title_idx ON tracks USING btree (title);
CREATE INDEX tracks_artist_idx ON tracks USING btree (artist);

CREATE TABLE requests (
    id serial PRIMARY KEY,
    telegram_id bigint NOT NULL,
    chat_id bigint NOT NULL,
    provider varchar NOT NULL,
    track_id text NOT NULL,
    is_cache_hit boolean NOT NULL,
    duration_ms integer,
    status text NOT NULL,
    error_reason text,
    created_at timestamptz DEFAULT now() NOT NULL
);
ALTER TABLE requests
    ADD CONSTRAINT requests_provider_check CHECK (provider IN ('apple', 'spotify', 'amazon'));
CREATE INDEX requests_provider_track_id_idx ON requests (provider, track_id);
CREATE INDEX requests_telegram_id_idx ON requests (telegram_id);
CREATE INDEX requests_created_at_idx ON requests (created_at);

CREATE TABLE settings (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    ripping_mode text NOT NULL DEFAULT 'live'
        CHECK (ripping_mode IN ('live', 'cache_only', 'paused')),
    album_rip_enabled boolean NOT NULL DEFAULT true,
    playlist_rip_enabled boolean NOT NULL DEFAULT true,
    artist_rip_enabled boolean NOT NULL DEFAULT true,
    txt_rip_enabled boolean NOT NULL DEFAULT true,
    multi_link_rip_enabled boolean NOT NULL DEFAULT true,
    max_collection_tracks integer NOT NULL DEFAULT 50
        CHECK (max_collection_tracks >= 0),
    auto_dump_enabled boolean NOT NULL DEFAULT true,
    auto_dump_storefronts text[] NOT NULL DEFAULT ARRAY['us']::text[],
    updated_at timestamptz DEFAULT now() NOT NULL
);

INSERT INTO settings (id) VALUES (1);

CREATE INDEX tracks_search_trgm_idx
    ON tracks USING gin ((title || ' ' || artist || ' ' || album) gin_trgm_ops);
