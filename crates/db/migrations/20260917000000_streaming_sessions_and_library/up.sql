-- Streaming sessions, one-time codes, favorites, and playlists schema

CREATE TABLE user_sessions (
    id TEXT PRIMARY KEY,
    telegram_id BIGINT NOT NULL REFERENCES users(telegram_id) ON DELETE CASCADE,
    refresh_token_hash TEXT NOT NULL UNIQUE,
    device_name TEXT,
    platform TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX idx_user_sessions_telegram_id ON user_sessions(telegram_id);
CREATE INDEX idx_user_sessions_refresh_token_hash ON user_sessions(refresh_token_hash);

CREATE TABLE one_time_auth_codes (
    code VARCHAR(32) PRIMARY KEY,
    telegram_id BIGINT NOT NULL REFERENCES users(telegram_id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_one_time_auth_codes_telegram_id ON one_time_auth_codes(telegram_id);

CREATE TABLE user_favorites (
    telegram_id BIGINT NOT NULL REFERENCES users(telegram_id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (telegram_id, track_id)
);

CREATE INDEX idx_user_favorites_telegram_id ON user_favorites(telegram_id);
CREATE INDEX idx_user_favorites_track_id ON user_favorites(track_id);

CREATE TABLE user_playlists (
    id SERIAL PRIMARY KEY,
    telegram_id BIGINT NOT NULL REFERENCES users(telegram_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_playlists_telegram_id ON user_playlists(telegram_id);

CREATE TABLE user_playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES user_playlists(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (playlist_id, track_id)
);

CREATE INDEX idx_user_playlist_tracks_playlist_pos ON user_playlist_tracks(playlist_id, position);
