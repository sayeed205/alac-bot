ALTER TABLE settings
    ADD COLUMN ripping_mode text NOT NULL DEFAULT 'live',
    ADD COLUMN album_rip_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN playlist_rip_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN artist_rip_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN txt_rip_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN multi_link_rip_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN max_collection_tracks integer NOT NULL DEFAULT 50,
    ADD COLUMN auto_dump_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN auto_dump_storefronts text[] NOT NULL DEFAULT ARRAY['us']::text[];

UPDATE settings SET
    ripping_mode = COALESCE(data->>'ripping_mode', 'live'),
    album_rip_enabled = COALESCE((data->>'album_rip_enabled')::boolean, true),
    playlist_rip_enabled = COALESCE((data->>'playlist_rip_enabled')::boolean, true),
    artist_rip_enabled = COALESCE((data->>'artist_rip_enabled')::boolean, true),
    txt_rip_enabled = COALESCE((data->>'txt_rip_enabled')::boolean, true),
    multi_link_rip_enabled = COALESCE((data->>'multi_link_rip_enabled')::boolean, true),
    max_collection_tracks = COALESCE((data->>'max_collection_tracks')::integer, 50),
    auto_dump_enabled = COALESCE((data->>'auto_dump_enabled')::boolean, true),
    auto_dump_storefronts = ARRAY(SELECT jsonb_array_elements_text(COALESCE(data->'auto_dump_storefronts', '["us"]'::jsonb)));

ALTER TABLE settings DROP COLUMN data;
