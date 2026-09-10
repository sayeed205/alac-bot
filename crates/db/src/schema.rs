diesel::table! {
    users (telegram_id) {
        telegram_id -> BigInt,
        name -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    tracks (id) {
        id -> Integer,
        provider -> Varchar,
        track_id -> Text,
        message_id -> Integer,
        file_id -> Text,
        file_unique_id -> Text,
        title -> Text,
        artist -> Text,
        album -> Text,
        duration -> Integer,
        bit_depth -> Integer,
        sample_rate -> Integer,
        genre -> Text,
        release_date -> Text,
        track_number -> Integer,
        track_count -> Integer,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    requests (id) {
        id -> Integer,
        telegram_id -> BigInt,
        chat_id -> BigInt,
        provider -> Varchar,
        track_id -> Text,
        is_cache_hit -> Bool,
        duration_ms -> Nullable<Integer>,
        status -> Text,
        error_reason -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    settings (id) {
        id -> SmallInt,
        ripping_mode -> Text,
        album_rip_enabled -> Bool,
        playlist_rip_enabled -> Bool,
        artist_rip_enabled -> Bool,
        txt_rip_enabled -> Bool,
        multi_link_rip_enabled -> Bool,
        max_collection_tracks -> Integer,
        auto_dump_enabled -> Bool,
        auto_dump_storefronts -> Array<Text>,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    albums (id) {
        id -> Integer,
        provider -> Varchar,
        album_id -> Text,
        part_index -> Integer,
        total_parts -> Integer,
        message_id -> Integer,
        file_id -> Text,
        file_unique_id -> Text,
        file_size -> BigInt,
        file_name -> Text,
        generation_hash -> Varchar,
        created_at -> Timestamptz,
    }
}

diesel::allow_tables_to_appear_in_same_query!(users, tracks, requests, settings, albums);
