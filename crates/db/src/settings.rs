use std::sync::RwLock;

use engine::settings::{BotSettings, RippingMode, DEFAULT_SETTINGS};
use serde_json::{json, Value};
use welds::connections::{Client, Param};

use crate::DbError;

fn defaults() -> BotSettings {
    BotSettings {
        auto_dump_storefronts: vec!["us".to_owned()],
        ..DEFAULT_SETTINGS.clone()
    }
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        // JavaScript considers arrays and objects truthy, including empty ones.
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) => "".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn js_number(value: &Value) -> Option<f64> {
    match value {
        Value::Null => Some(0.0),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse().ok(),
        Value::Array(values) if values.is_empty() => Some(0.0),
        Value::Array(values) if values.len() == 1 => js_number(&values[0]),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn as_u32(value: &Value) -> Option<u32> {
    let number = js_number(value)?;
    if number.is_finite() && number >= 0.0 && number <= f64::from(u32::MAX) {
        Some(number as u32)
    } else {
        None
    }
}

fn key_name(key: &str) -> Option<&'static str> {
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

/// Write-through key/value settings store with an in-memory snapshot.
pub struct SettingsStore {
    client: welds::connections::postgres::PostgresClient,
    cached_settings: RwLock<BotSettings>,
}

impl SettingsStore {
    pub fn new(client: welds::connections::postgres::PostgresClient) -> Self {
        Self {
            client,
            cached_settings: RwLock::new(defaults()),
        }
    }

    pub async fn init(&self) {
        let result = self.load().await;
        let mut cache = self
            .cached_settings
            .write()
            .expect("settings lock poisoned");
        *cache = result.unwrap_or_else(|_| defaults());
    }

    async fn load(&self) -> Result<BotSettings, DbError> {
        let rows = self
            .client
            .fetch_rows("SELECT key, value FROM settings", &[])
            .await?;
        let mut settings = defaults();
        for row in rows {
            let key: String = row
                .get("key")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let value: Value = row
                .get("value")
                .map_err(|error| DbError::Row(error.to_string()))?;
            match key.as_str() {
                "ripping_mode" => {
                    if let Value::String(value) = value {
                        if let Some(mode) = RippingMode::parse(&value) {
                            settings.ripping_mode = mode;
                        }
                    }
                }
                "album_rip_enabled" => settings.album_rip_enabled = js_truthy(&value),
                "playlist_rip_enabled" => settings.playlist_rip_enabled = js_truthy(&value),
                "artist_rip_enabled" => settings.artist_rip_enabled = js_truthy(&value),
                "txt_rip_enabled" => settings.txt_rip_enabled = js_truthy(&value),
                "multi_link_rip_enabled" => settings.multi_link_rip_enabled = js_truthy(&value),
                "max_collection_tracks" => {
                    if let Some(value) = as_u32(&value) {
                        settings.max_collection_tracks = value;
                    }
                }
                "auto_dump_enabled" => settings.auto_dump_enabled = js_truthy(&value),
                "auto_dump_storefronts" => {
                    if let Value::Array(values) = value {
                        let values: Vec<String> = values
                            .iter()
                            .map(js_string)
                            .map(|value| value.to_lowercase().trim().to_owned())
                            .filter(|value| !value.is_empty())
                            .collect();
                        if !values.is_empty() {
                            settings.auto_dump_storefronts = values;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn get_settings(&self) -> BotSettings {
        self.cached_settings
            .read()
            .expect("settings lock poisoned")
            .clone()
    }

    pub async fn set_setting(&self, key: &str, value: Value) -> BotSettings {
        let Some(db_key) = key_name(key) else {
            return self.get_settings();
        };
        {
            let mut settings = self
                .cached_settings
                .write()
                .expect("settings lock poisoned");
            apply_set_value(&mut settings, db_key, &value);
        }
        let params: [&(dyn Param + Sync); 2] = [&db_key, &value];
        if let Err(error) = self
            .client
            .execute(
                "INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
                &params,
            )
            .await
        {
            tracing::error!(key = db_key, error = %error, "failed to persist setting");
        }
        self.get_settings()
    }

    pub async fn cycle_ripping_mode(&self) -> RippingMode {
        let next = match self.get_settings().ripping_mode {
            RippingMode::Live => RippingMode::CacheOnly,
            RippingMode::CacheOnly => RippingMode::Paused,
            RippingMode::Paused => RippingMode::Live,
        };
        self.set_setting("ripping_mode", json!(next.as_str())).await;
        next
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
        let value = match key {
            "album_rip_enabled" => !self.get_settings().album_rip_enabled,
            "playlist_rip_enabled" => !self.get_settings().playlist_rip_enabled,
            "artist_rip_enabled" => !self.get_settings().artist_rip_enabled,
            "txt_rip_enabled" => !self.get_settings().txt_rip_enabled,
            "auto_dump_enabled" => !self.get_settings().auto_dump_enabled,
            _ => !self.get_settings().multi_link_rip_enabled,
        };
        self.set_setting(key, json!(value)).await;
        value
    }

    pub async fn set_max_collection_tracks(&self, limit: i64) -> u32 {
        let value = limit.max(0) as u32;
        self.set_setting("max_collection_tracks", json!(value))
            .await;
        value
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
        let mut values: Vec<String> = self
            .get_settings()
            .auto_dump_storefronts
            .into_iter()
            .filter(|value| value != &clean)
            .collect();
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

fn apply_value(settings: &mut BotSettings, key: &str, value: &Value) {
    match key {
        "ripping_mode" => {
            if let Value::String(value) = value {
                if let Some(mode) = RippingMode::parse(value) {
                    settings.ripping_mode = mode;
                }
            }
        }
        "album_rip_enabled" => settings.album_rip_enabled = js_truthy(value),
        "playlist_rip_enabled" => settings.playlist_rip_enabled = js_truthy(value),
        "artist_rip_enabled" => settings.artist_rip_enabled = js_truthy(value),
        "txt_rip_enabled" => settings.txt_rip_enabled = js_truthy(value),
        "multi_link_rip_enabled" => settings.multi_link_rip_enabled = js_truthy(value),
        "max_collection_tracks" => {
            if let Some(value) = as_u32(value) {
                settings.max_collection_tracks = value;
            }
        }
        "auto_dump_enabled" => settings.auto_dump_enabled = js_truthy(value),
        "auto_dump_storefronts" => {
            if let Value::Array(values) = value {
                let values: Vec<String> = values
                    .iter()
                    .map(js_string)
                    .map(|value| value.to_lowercase().trim().to_owned())
                    .filter(|value| !value.is_empty())
                    .collect();
                if !values.is_empty() {
                    settings.auto_dump_storefronts = values;
                }
            }
        }
        _ => {}
    }
}

fn apply_set_value(settings: &mut BotSettings, key: &str, value: &Value) {
    if key == "auto_dump_storefronts" {
        if let Value::Array(values) = value {
            settings.auto_dump_storefronts = values.iter().map(js_string).collect();
        }
    } else {
        apply_value(settings, key, value);
    }
}
