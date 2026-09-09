use std::sync::RwLock;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use engine::settings::{default_settings, BotSettings, RippingMode};
use serde_json::{json, Value};

use crate::{models::SettingsRow, schema::settings, DbError, DbPool};

/// The settings table is a one-row, typed configuration record.  The JSON
/// value accepted by `set_setting` remains as the service boundary used by
/// the bot, but it is decoded once and never stored as untyped JSON.
pub struct SettingsStore {
    pool: DbPool,
    cached_settings: RwLock<BotSettings>,
}

impl SettingsStore {
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            cached_settings: RwLock::new(default_settings()),
        }
    }

    pub async fn init(&self) -> Result<(), DbError> {
        self.reload().await
    }

    /// Reload the cache from PostgreSQL.  This is intentionally fallible: a
    /// stale in-memory snapshot is unsafe after a restore or a reconnect.
    pub async fn reload(&self) -> Result<(), DbError> {
        let result = self.load().await?;
        *self
            .cached_settings
            .write()
            .expect("settings lock poisoned") = result;
        Ok(())
    }

    async fn load(&self) -> Result<BotSettings, DbError> {
        let mut connection = self.pool.connection().await?;
        let row = settings::table
            .filter(settings::id.eq(1_i16))
            .select(SettingsRow::as_select())
            .first::<SettingsRow>(&mut *connection)
            .await?;
        Ok(from_row(row))
    }

    pub fn get_settings(&self) -> BotSettings {
        self.cached_settings
            .read()
            .expect("settings lock poisoned")
            .clone()
    }

    /// Set one externally named setting while persisting the complete typed
    /// singleton. Unknown or incorrectly typed values are ignored, matching
    /// the service's previous non-throwing boundary without coercion.
    pub async fn set_setting(&self, key: &str, value: Value) -> BotSettings {
        let key = canonical_key(key);
        let Some(key) = key else {
            return self.get_settings();
        };
        let mut next = self.get_settings();
        if !apply_value(&mut next, key, &value) {
            return next;
        }
        if let Err(error) = self.persist(&next).await {
            tracing::error!(%error, setting = key, "failed to persist setting");
            return self.get_settings();
        }
        *self
            .cached_settings
            .write()
            .expect("settings lock poisoned") = next.clone();
        next
    }

    async fn persist(&self, value: &BotSettings) -> Result<(), DbError> {
        let mut connection = self.pool.connection().await?;
        diesel::update(settings::table.filter(settings::id.eq(1_i16)))
            .set((
                settings::ripping_mode.eq(value.ripping_mode.as_str()),
                settings::album_rip_enabled.eq(value.album_rip_enabled),
                settings::playlist_rip_enabled.eq(value.playlist_rip_enabled),
                settings::artist_rip_enabled.eq(value.artist_rip_enabled),
                settings::txt_rip_enabled.eq(value.txt_rip_enabled),
                settings::multi_link_rip_enabled.eq(value.multi_link_rip_enabled),
                settings::max_collection_tracks.eq(i32::try_from(value.max_collection_tracks)
                    .map_err(|error| DbError::Row(error.to_string()))?),
                settings::auto_dump_enabled.eq(value.auto_dump_enabled),
                settings::auto_dump_storefronts.eq(&value.auto_dump_storefronts),
                settings::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    pub async fn cycle_ripping_mode(&self) -> RippingMode {
        let next = self.get_settings().cycled_mode();
        self.set_setting("ripping_mode", json!(next.as_str()))
            .await
            .ripping_mode
    }

    pub async fn toggle_album(&self) -> bool {
        self.toggle("album_rip_enabled").await
    }
    pub async fn toggle_playlist(&self) -> bool {
        self.toggle("playlist_rip_enabled").await
    }
    pub async fn toggle_artist(&self) -> bool {
        self.toggle("artist_rip_enabled").await
    }
    pub async fn toggle_txt(&self) -> bool {
        self.toggle("txt_rip_enabled").await
    }
    pub async fn toggle_multi_link_rip(&self) -> bool {
        self.toggle("multi_link_rip_enabled").await
    }
    pub async fn toggle_auto_dump(&self) -> bool {
        self.toggle("auto_dump_enabled").await
    }

    async fn toggle(&self, key: &str) -> bool {
        let current = self.get_settings();
        let value = match key {
            "album_rip_enabled" => !current.album_rip_enabled,
            "playlist_rip_enabled" => !current.playlist_rip_enabled,
            "artist_rip_enabled" => !current.artist_rip_enabled,
            "txt_rip_enabled" => !current.txt_rip_enabled,
            "auto_dump_enabled" => !current.auto_dump_enabled,
            _ => !current.multi_link_rip_enabled,
        };
        let settings = self.set_setting(key, json!(value)).await;
        match key {
            "album_rip_enabled" => settings.album_rip_enabled,
            "playlist_rip_enabled" => settings.playlist_rip_enabled,
            "artist_rip_enabled" => settings.artist_rip_enabled,
            "txt_rip_enabled" => settings.txt_rip_enabled,
            "auto_dump_enabled" => settings.auto_dump_enabled,
            _ => settings.multi_link_rip_enabled,
        }
    }

    pub async fn set_max_collection_tracks(&self, limit: i64) -> u32 {
        let value = u32::try_from(limit.max(0))
            .unwrap_or(engine::limits::MAX_COLLECTION_TRACKS)
            .min(engine::limits::MAX_COLLECTION_TRACKS);
        self.set_setting("max_collection_tracks", json!(value))
            .await
            .max_collection_tracks
    }

    pub async fn add_auto_dump_storefront(&self, storefront: &str) -> Vec<String> {
        let clean = storefront.to_lowercase().trim().to_owned();
        let mut values = self.get_settings().auto_dump_storefronts;
        if clean.is_empty() || values.iter().any(|value| value == &clean) {
            return values;
        }
        values.push(clean);
        self.set_setting("auto_dump_storefronts", json!(values))
            .await
            .auto_dump_storefronts
    }

    pub async fn remove_auto_dump_storefront(&self, storefront: &str) -> Vec<String> {
        let clean = storefront.to_lowercase().trim().to_owned();
        let mut values = self
            .get_settings()
            .auto_dump_storefronts
            .into_iter()
            .filter(|value| value != &clean)
            .collect::<Vec<_>>();
        if values.is_empty() {
            values.push("us".to_owned());
        }
        self.set_setting("auto_dump_storefronts", json!(values))
            .await
            .auto_dump_storefronts
    }

    pub async fn set_auto_dump_storefronts(&self, storefronts: &[String]) -> Vec<String> {
        let mut values = Vec::new();
        for storefront in storefronts {
            let clean = storefront.to_lowercase().trim().to_owned();
            if !clean.is_empty() && !values.contains(&clean) {
                values.push(clean);
            }
        }
        if values.is_empty() {
            values.push("us".to_owned());
        }
        self.set_setting("auto_dump_storefronts", json!(values))
            .await
            .auto_dump_storefronts
    }
}

fn from_row(row: SettingsRow) -> BotSettings {
    BotSettings {
        ripping_mode: RippingMode::parse(&row.ripping_mode).unwrap_or_default(),
        album_rip_enabled: row.album_rip_enabled,
        playlist_rip_enabled: row.playlist_rip_enabled,
        artist_rip_enabled: row.artist_rip_enabled,
        txt_rip_enabled: row.txt_rip_enabled,
        multi_link_rip_enabled: row.multi_link_rip_enabled,
        max_collection_tracks: u32::try_from(row.max_collection_tracks)
            .ok()
            .filter(|value| engine::limits::validate_collection_limit(*value))
            .unwrap_or(50),
        auto_dump_enabled: row.auto_dump_enabled,
        auto_dump_storefronts: if row.auto_dump_storefronts.is_empty() {
            vec!["us".to_owned()]
        } else {
            row.auto_dump_storefronts
        },
    }
}

fn canonical_key(key: &str) -> Option<&'static str> {
    match key {
        "ripping_mode" | "rippingMode" => Some("ripping_mode"),
        "album_rip_enabled" | "albumRipEnabled" => Some("album_rip_enabled"),
        "playlist_rip_enabled" | "playlistRipEnabled" => Some("playlist_rip_enabled"),
        "artist_rip_enabled" | "artistRipEnabled" => Some("artist_rip_enabled"),
        "txt_rip_enabled" | "txtRipEnabled" => Some("txt_rip_enabled"),
        "multi_link_rip_enabled" | "multiLinkRipEnabled" => Some("multi_link_rip_enabled"),
        "max_collection_tracks" | "maxCollectionTracks" => Some("max_collection_tracks"),
        "auto_dump_enabled" | "autoDumpEnabled" => Some("auto_dump_enabled"),
        "auto_dump_storefronts" | "autoDumpStorefronts" => Some("auto_dump_storefronts"),
        _ => None,
    }
}

fn apply_value(settings: &mut BotSettings, key: &str, value: &Value) -> bool {
    match key {
        "ripping_mode" => value
            .as_str()
            .and_then(RippingMode::parse)
            .map(|mode| settings.ripping_mode = mode)
            .is_some(),
        "album_rip_enabled" => value
            .as_bool()
            .map(|v| settings.album_rip_enabled = v)
            .is_some(),
        "playlist_rip_enabled" => value
            .as_bool()
            .map(|v| settings.playlist_rip_enabled = v)
            .is_some(),
        "artist_rip_enabled" => value
            .as_bool()
            .map(|v| settings.artist_rip_enabled = v)
            .is_some(),
        "txt_rip_enabled" => value
            .as_bool()
            .map(|v| settings.txt_rip_enabled = v)
            .is_some(),
        "multi_link_rip_enabled" => value
            .as_bool()
            .map(|v| settings.multi_link_rip_enabled = v)
            .is_some(),
        "max_collection_tracks" => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| engine::limits::validate_collection_limit(*v))
            .map(|v| settings.max_collection_tracks = v)
            .is_some(),
        "auto_dump_enabled" => value
            .as_bool()
            .map(|v| settings.auto_dump_enabled = v)
            .is_some(),
        "auto_dump_storefronts" => value
            .as_array()
            .map(|values| {
                let cleaned = values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|v| v.to_lowercase().trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect::<Vec<_>>();
                if !cleaned.is_empty() {
                    settings.auto_dump_storefronts = cleaned;
                }
            })
            .is_some(),
        _ => false,
    }
}
