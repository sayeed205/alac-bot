-- Rollback streaming sessions, auth codes, favorites, and playlists

DROP TABLE IF EXISTS user_playlist_tracks;
DROP TABLE IF EXISTS user_playlists;
DROP TABLE IF EXISTS user_favorites;
DROP TABLE IF EXISTS one_time_auth_codes;
DROP TABLE IF EXISTS user_sessions;
