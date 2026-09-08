//! Bot settings: ripping mode + feature flags (port of
//! `src/modules/settings/types.ts` + the pure logic of `service.ts`).
//!
//! The DB-backed `SettingsService` (key-value rows, write-through cache)
//! lives in the db crate (M5b); this module carries the domain types,
//! defaults, and permission logic with exact TS parity.

/// Whether the bot rips live, serves cache only, or is paused.
/// TS: `'live' | 'cache_only' | 'paused'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RippingMode {
    #[default]
    Live,
    CacheOnly,
    Paused,
}

impl RippingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RippingMode::Live => "live",
            RippingMode::CacheOnly => "cache_only",
            RippingMode::Paused => "paused",
        }
    }

    /// Parse a stored `ripping_mode` row value; anything else falls back to
    /// the default (TS keeps the default when the value is not one of the
    /// three literals).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "live" => Some(RippingMode::Live),
            "cache_only" => Some(RippingMode::CacheOnly),
            "paused" => Some(RippingMode::Paused),
            _ => None,
        }
    }
}

/// TS `BotSettings` shape (settings/types.ts).
#[derive(Debug, Clone, PartialEq)]
pub struct BotSettings {
    pub ripping_mode: RippingMode,
    pub album_rip_enabled: bool,
    pub playlist_rip_enabled: bool,
    pub artist_rip_enabled: bool,
    pub txt_rip_enabled: bool,
    pub multi_link_rip_enabled: bool,
    pub max_collection_tracks: u32,
    pub auto_dump_enabled: bool,
    pub auto_dump_storefronts: Vec<String>,
}

/// TS `DEFAULT_SETTINGS` (settings/types.ts lines 22-32).
pub const DEFAULT_SETTINGS: BotSettings = BotSettings {
    ripping_mode: RippingMode::Live,
    album_rip_enabled: true,
    playlist_rip_enabled: true,
    artist_rip_enabled: true,
    txt_rip_enabled: true,
    multi_link_rip_enabled: true,
    max_collection_tracks: 50,
    auto_dump_enabled: true,
    auto_dump_storefronts: Vec::new(), // filled by `default_settings()` — see below
};

impl BotSettings {
    /// `canRipLive(isAdmin)`: admins always pass; users need mode `'live'`.
    pub fn can_rip_live(&self, is_admin: bool) -> bool {
        if is_admin {
            return true;
        }
        self.ripping_mode == RippingMode::Live
    }

    /// `canServeCache(isAdmin)`: admins always pass; users need mode
    /// `!= 'paused'`.
    pub fn can_serve_cache(&self, is_admin: bool) -> bool {
        if is_admin {
            return true;
        }
        self.ripping_mode != RippingMode::Paused
    }

    pub fn can_rip_album(&self, is_admin: bool) -> bool {
        is_admin || self.album_rip_enabled
    }

    pub fn can_rip_playlist(&self, is_admin: bool) -> bool {
        is_admin || self.playlist_rip_enabled
    }

    pub fn can_rip_artist(&self, is_admin: bool) -> bool {
        is_admin || self.artist_rip_enabled
    }

    pub fn can_rip_txt(&self, is_admin: bool) -> bool {
        is_admin || self.txt_rip_enabled
    }

    pub fn can_rip_multi_link(&self, is_admin: bool) -> bool {
        is_admin || self.multi_link_rip_enabled
    }

    /// TS `cycleRippingMode`: live → cache_only → paused → live.
    pub fn cycled_mode(&self) -> RippingMode {
        match self.ripping_mode {
            RippingMode::Live => RippingMode::CacheOnly,
            RippingMode::CacheOnly => RippingMode::Paused,
            RippingMode::Paused => RippingMode::Live,
        }
    }
}

/// `DEFAULT_SETTINGS` with its `autoDumpStorefronts: ['us']` — the const
/// cannot hold a `Vec<String>`, so callers use this constructor.
pub fn default_settings() -> BotSettings {
    BotSettings {
        auto_dump_storefronts: vec!["us".to_string()],
        ..DEFAULT_SETTINGS.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts() {
        let d = default_settings();
        assert_eq!(d.ripping_mode, RippingMode::Live);
        assert!(d.album_rip_enabled);
        assert!(d.playlist_rip_enabled);
        assert!(d.artist_rip_enabled);
        assert!(d.txt_rip_enabled);
        assert!(d.multi_link_rip_enabled);
        assert_eq!(d.max_collection_tracks, 50);
        assert!(d.auto_dump_enabled);
        assert_eq!(d.auto_dump_storefronts, vec!["us".to_string()]);
    }

    #[test]
    fn mode_parse_roundtrip() {
        assert_eq!(RippingMode::parse("live"), Some(RippingMode::Live));
        assert_eq!(
            RippingMode::parse("cache_only"),
            Some(RippingMode::CacheOnly)
        );
        assert_eq!(RippingMode::parse("paused"), Some(RippingMode::Paused));
        assert_eq!(RippingMode::parse("bogus"), None);
    }

    #[test]
    fn can_rip_live_admin_bypass_and_modes() {
        let live = default_settings();
        let cache = BotSettings {
            ripping_mode: RippingMode::CacheOnly,
            ..default_settings()
        };
        let paused = BotSettings {
            ripping_mode: RippingMode::Paused,
            ..default_settings()
        };

        assert!(live.can_rip_live(false));
        assert!(!cache.can_rip_live(false));
        assert!(!paused.can_rip_live(false));
        // Admins bypass everything.
        assert!(paused.can_rip_live(true));
    }

    #[test]
    fn can_serve_cache_blocks_only_when_paused() {
        let live = default_settings();
        let cache = BotSettings {
            ripping_mode: RippingMode::CacheOnly,
            ..default_settings()
        };
        let paused = BotSettings {
            ripping_mode: RippingMode::Paused,
            ..default_settings()
        };

        assert!(live.can_serve_cache(false));
        assert!(cache.can_serve_cache(false));
        assert!(!paused.can_serve_cache(false));
        assert!(paused.can_serve_cache(true));
    }

    #[test]
    fn flag_gates_respect_admin_bypass() {
        let all_off = BotSettings {
            album_rip_enabled: false,
            playlist_rip_enabled: false,
            artist_rip_enabled: false,
            txt_rip_enabled: false,
            multi_link_rip_enabled: false,
            ..default_settings()
        };

        assert!(!all_off.can_rip_album(false));
        assert!(!all_off.can_rip_playlist(false));
        assert!(!all_off.can_rip_artist(false));
        assert!(!all_off.can_rip_txt(false));
        assert!(!all_off.can_rip_multi_link(false));
        assert!(all_off.can_rip_album(true));
        assert!(all_off.can_rip_playlist(true));
        assert!(all_off.can_rip_artist(true));
        assert!(all_off.can_rip_txt(true));
        assert!(all_off.can_rip_multi_link(true));
    }

    #[test]
    fn mode_cycles_like_ts() {
        let live = default_settings();
        assert_eq!(live.cycled_mode(), RippingMode::CacheOnly);

        let cache = BotSettings {
            ripping_mode: RippingMode::CacheOnly,
            ..default_settings()
        };
        assert_eq!(cache.cycled_mode(), RippingMode::Paused);

        let paused = BotSettings {
            ripping_mode: RippingMode::Paused,
            ..default_settings()
        };
        assert_eq!(paused.cycled_mode(), RippingMode::Live);
    }
}
