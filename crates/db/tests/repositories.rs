use std::time::Duration;

use db::{migrate, RequestLogRepository, SettingsStore, TracksRepository};
use engine::orchestrator::deps::{RequestLog, SaveTrackInput};
use serde_json::json;
use welds::connections::{Client, Param};

async fn client() -> welds::connections::postgres::PostgresClient {
    let url = "postgresql://admin:password@localhost:5432/alac_bot_v2_test";
    let client = tokio::time::timeout(
        Duration::from_secs(3),
        welds::connections::postgres::connect(url),
    )
    .await
    .expect("database connection timed out")
    .expect("database connection failed");
    migrate(&client).await.expect("migration");
    client
}

fn track(id: &str, title: &str) -> SaveTrackInput {
    SaveTrackInput {
        apple_track_id: id.to_owned(),
        message_id: 123,
        file_id: format!("file-{id}"),
        file_unique_id: format!("unique-{id}"),
        title: title.to_owned(),
        artist: "Test Artist".to_owned(),
        album: "Test Album".to_owned(),
        duration: 180,
        bit_depth: 24,
        sample_rate: 44_100,
        genre: "Test".to_owned(),
        release_date: "2026-01-01".to_owned(),
        track_number: 1,
        track_count: 1,
    }
}

async fn clean_tracks(client: &welds::connections::postgres::PostgresClient, prefix: &str) {
    client
        .execute(
            "DELETE FROM tracks WHERE apple_track_id LIKE $1",
            &[&format!("{prefix}%") as &(dyn Param + Sync)],
        )
        .await
        .expect("track cleanup");
}

#[tokio::test]
async fn track_cache_hit_miss_and_empty_list() {
    let client = client().await;
    let prefix = format!("db-m5b-cache-{}-", std::process::id());
    clean_tracks(&client, &prefix).await;
    let repository = TracksRepository::new(client.clone());
    let id = format!("{prefix}hit");
    repository
        .save_track(&track(&id, "Cache Hit"))
        .await
        .expect("save");

    let result = repository
        .find_cached_tracks(&[id.clone(), format!("{prefix}miss"), String::new()])
        .await
        .expect("find cached tracks");
    assert_eq!(result.len(), 1);
    assert_eq!(result[&id].title, "Cache Hit");
    assert!(repository
        .find_cached_tracks(&[])
        .await
        .expect("empty lookup")
        .is_empty());
    assert!(repository
        .find_track_by_file_unique_id(&format!("unique-{id}"))
        .await
        .expect("file unique lookup")
        .is_some());
    clean_tracks(&client, &prefix).await;
}

#[tokio::test]
async fn save_find_delete_search_and_prune_tracks() {
    let client = client().await;
    let prefix = format!("db-m5b-ops-{}-", std::process::id());
    clean_tracks(&client, &prefix).await;
    let repository = TracksRepository::new(client.clone());
    let first = format!("{prefix}first");
    let second = format!("{prefix}second");
    repository
        .save_track(&track(&first, "A Unique Song"))
        .await
        .expect("save first");
    repository
        .save_track(&track(&second, "Another Song"))
        .await
        .expect("save second");
    assert_eq!(
        repository
            .search_cached_tracks("unique song", 10)
            .await
            .expect("search")
            .len(),
        1
    );
    let ids = repository.get_all_track_ids().await.expect("all ids");
    assert!(ids.contains(&first) && ids.contains(&second));
    assert_eq!(
        repository
            .delete_tracks_not_in(std::slice::from_ref(&first))
            .await
            .expect("prune"),
        1
    );
    assert!(repository.delete_track(&first).await.expect("delete hit"));
    assert!(!repository.delete_track(&first).await.expect("delete miss"));
    clean_tracks(&client, &prefix).await;
}

#[tokio::test]
async fn request_log_insert() {
    let client = client().await;
    let repository = RequestLogRepository::new(client.clone());
    let id = format!("db-m5b-request-{}", std::process::id());
    client
        .execute(
            "DELETE FROM requests WHERE apple_track_id = $1",
            &[&id as &(dyn Param + Sync)],
        )
        .await
        .expect("request cleanup");
    repository
        .log_request(&RequestLog {
            telegram_id: 9_001,
            chat_id: -9_002,
            apple_track_id: id.clone(),
            is_cache_hit: false,
            duration_ms: Some(42),
            status: "ok".to_owned(),
            error_reason: None,
        })
        .await
        .expect("log request");
    let rows = client
        .fetch_rows(
            "SELECT status FROM requests WHERE apple_track_id = $1",
            &[&id as &(dyn Param + Sync)],
        )
        .await
        .expect("request query");
    assert_eq!(rows.len(), 1);
    client
        .execute(
            "DELETE FROM requests WHERE apple_track_id = $1",
            &[&id as &(dyn Param + Sync)],
        )
        .await
        .expect("request cleanup");
}

#[tokio::test]
async fn settings_defaults_parsing_and_mutations() {
    let client = client().await;
    // Clean every key this test may write: the parsing probes below AND the
    // toggle mutations later in the test (which persist via set_setting).
    // Without this, a second run against the same DB reads the previous
    // run's toggled rows and fails.
    let keys = [
        "ripping_mode",
        "max_collection_tracks",
        "auto_dump_storefronts",
        "album_rip_enabled",
        "playlist_rip_enabled",
        "artist_rip_enabled",
        "txt_rip_enabled",
        "multi_link_rip_enabled",
        "non_array_storefronts",
    ];
    for key in keys {
        client
            .execute(
                "DELETE FROM settings WHERE key = $1",
                &[&key as &(dyn Param + Sync)],
            )
            .await
            .expect("settings cleanup");
    }
    let store = SettingsStore::new(client.clone());
    store.init().await;
    assert_eq!(store.get_settings().max_collection_tracks, 50);
    assert_eq!(store.get_settings().auto_dump_storefronts, vec!["us"]);

    let values = [
        ("ripping_mode", json!("not-a-mode")),
        ("max_collection_tracks", json!(-1)),
        ("auto_dump_storefronts", json!([])),
        ("album_rip_enabled", json!(false)),
    ];
    for (key, value) in values {
        client
            .execute(
                "INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                &[&key as &(dyn Param + Sync), &value as &(dyn Param + Sync)],
            )
            .await
            .expect("settings insert");
    }
    client
        .execute(
            "INSERT INTO settings (key, value) VALUES ('non_array_storefronts', 'null'::jsonb) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            &[],
        )
        .await
        .expect("non-array insert");
    store.init().await;
    assert_eq!(store.get_settings().ripping_mode.as_str(), "live");
    assert_eq!(store.get_settings().max_collection_tracks, 50);
    assert_eq!(store.get_settings().auto_dump_storefronts, vec!["us"]);
    assert!(!store.get_settings().album_rip_enabled);

    assert!(store.toggle_album().await);
    assert!(!store.toggle_playlist().await);
    assert!(!store.toggle_artist().await);
    assert!(!store.toggle_txt().await);
    assert!(!store.toggle_multi_link_rip().await);
    assert_eq!(
        store
            .set_setting("max_collection_tracks", json!(12))
            .await
            .max_collection_tracks,
        12
    );
    assert_eq!(store.cycle_ripping_mode().await.as_str(), "cache_only");
    assert_eq!(store.cycle_ripping_mode().await.as_str(), "paused");
    assert_eq!(store.cycle_ripping_mode().await.as_str(), "live");
    assert_eq!(store.set_max_collection_tracks(-8).await, 0);
    assert_eq!(
        store.add_auto_dump_storefront(" CA ").await,
        vec!["us", "ca"]
    );
    assert_eq!(store.add_auto_dump_storefront("ca").await, vec!["us", "ca"]);
    assert_eq!(store.remove_auto_dump_storefront("US").await, vec!["ca"]);
    assert_eq!(store.remove_auto_dump_storefront("ca").await, vec!["us"]);
    assert_eq!(store.set_auto_dump_storefronts(&[]).await, vec!["us"]);

    for key in keys {
        client
            .execute(
                "DELETE FROM settings WHERE key = $1",
                &[&key as &(dyn Param + Sync)],
            )
            .await
            .expect("settings cleanup");
    }
}
