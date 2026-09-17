ALTER TABLE settings ADD COLUMN data jsonb;

UPDATE settings SET data = jsonb_build_object(
    'ripping_mode', ripping_mode,
    'album_rip_enabled', album_rip_enabled,
    'playlist_rip_enabled', playlist_rip_enabled,
    'artist_rip_enabled', artist_rip_enabled,
    'txt_rip_enabled', txt_rip_enabled,
    'multi_link_rip_enabled', multi_link_rip_enabled,
    'max_collection_tracks', max_collection_tracks,
    'auto_dump_enabled', auto_dump_enabled,
    'auto_dump_storefronts', to_jsonb(auto_dump_storefronts),
    'apple_rip_enabled', true,
    'qobuz_rip_enabled', true
);

ALTER TABLE settings
    ALTER COLUMN data SET NOT NULL,
    ALTER COLUMN data SET DEFAULT '{}'::jsonb,
    DROP COLUMN ripping_mode,
    DROP COLUMN album_rip_enabled,
    DROP COLUMN playlist_rip_enabled,
    DROP COLUMN artist_rip_enabled,
    DROP COLUMN txt_rip_enabled,
    DROP COLUMN multi_link_rip_enabled,
    DROP COLUMN max_collection_tracks,
    DROP COLUMN auto_dump_enabled,
    DROP COLUMN auto_dump_storefronts;
