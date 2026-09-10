//! Production composition of the engine's orchestration dependencies.

use std::{collections::HashMap, path::Path, sync::Arc};

use engine::{
    catalog::{Catalog, ReqwestTransport},
    orchestrator::deps::{
        CachedTrack, OrchestratorDeps, RequestLog, SaveTrackInput, SinkError, TelegramSink,
    },
    playlist::{PlaylistClient, PlaylistData, PlaylistError, ReqwestPlaylistHttp},
    ripper::{AlacTrackRipper, EngineRipperDeps, RipError, RipProgressCallback, RipperConfig},
    settings::BotSettings,
    types::{AlbumTracks, ArtistTracks, TrackKey, TrackRipResult},
};
use tokio_util::sync::CancellationToken;

use crate::telegram_sink::FerogramTelegramSink;

/// All environment-owned production dependencies used by the orchestrator.
pub struct RipDeps {
    sink: FerogramTelegramSink,
    tracks: db::TracksRepository,
    requests: db::RequestLogRepository,
    settings: db::SettingsStore,
    catalog: Catalog<ReqwestTransport>,
    playlist: PlaylistClient<ReqwestPlaylistHttp>,
    ripper: AlacTrackRipper,
    ripper_deps: EngineRipperDeps,
    /// Shared with `ripper_deps` so health probes observe the same circuit
    /// and cache state the ripper uses.
    mirror_policy: engine::streaming::MirrorPolicyManager<engine::streaming::ReqwestHttp>,
    upload_retry_base_ms: u64,
    max_retries: u32,
}

impl RipDeps {
    /// Build the production dependency graph and load the settings snapshot.
    pub async fn new(
        client: Arc<ferogram::Client>,
        dump_peer: ferogram::PeerRef,
        tracks: db::TracksRepository,
        requests: db::RequestLogRepository,
        settings: db::SettingsStore,
    ) -> Result<Self, SinkError> {
        settings
            .init()
            .await
            .map_err(|error| SinkError(format!("load settings: {error}")))?;

        let catalog = Catalog::new(ReqwestTransport::new());
        let ripper_catalog = Catalog::new(ReqwestTransport::new());
        // TS env schema (`src/env.ts`): the wrapper default is
        // `http://127.0.0.1:12340`; the mirror key is `ALAC_API_KEY`.
        let wrapper_url = Some(
            env_option("ALAC_WRAPPER_URL").unwrap_or_else(|| "http://127.0.0.1:12340".to_owned()),
        );
        let wrapper_api_key = env_option("ALAC_WRAPPER_API_KEY");
        let stream_transport =
            engine::streaming::StreamTransport::new(engine::streaming::ReqwestHttp::new());
        let mirror_policy = engine::streaming::MirrorPolicyManager::new(
            engine::streaming::ReqwestHttp::new(),
            env_option("ALAC_MIRROR_URL").zip(env_option("ALAC_API_KEY")),
        );
        // The ripper and the dashboard health probe share one policy state.
        let probe_policy = mirror_policy.shared();
        let ripper_deps = EngineRipperDeps::new(
            ripper_catalog,
            mirror_policy,
            stream_transport,
            wrapper_url,
            wrapper_api_key,
        );
        let retry_base_ms = std::env::var("ALAC_RETRY_BASE_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value <= engine::limits::MAX_RETRY_BASE_MS)
            // TS default: 2000ms (`src/env.ts` ALAC_RETRY_BASE_MS).
            .unwrap_or(2000);
        let max_retries = std::env::var("ALAC_MAX_RETRIES")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value <= engine::limits::MAX_RETRIES)
            .unwrap_or(3);
        let mut ripper_config = RipperConfig::default();
        ripper_config.base_delay_ms = retry_base_ms;
        ripper_config.max_retries = max_retries;

        Ok(Self {
            sink: FerogramTelegramSink::new(client, dump_peer).await?,
            tracks,
            requests,
            settings,
            catalog,
            playlist: PlaylistClient::new(ReqwestPlaylistHttp::new()),
            ripper: AlacTrackRipper::new(ripper_config),
            ripper_deps,
            mirror_policy: probe_policy,
            upload_retry_base_ms: retry_base_ms,
            max_retries,
        })
    }
}

fn env_option(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

impl RipDeps {
    /// Probe mirror health for the status dashboard (oracle
    /// `commands/health.ts:47-61`). Uses a clone of the ripper's policy
    /// manager, so circuit state and the endpoint cache stay coherent
    /// between rips and probes.
    pub async fn probe_mirror_health(&self) -> crate::mirror_health::HealthReport {
        use crate::mirror_health::{MirrorHealthProbe, PolicyProbe};

        PolicyProbe::new(self.mirror_policy.shared()).probe().await
    }

    /// The mutable settings store backing the admin `/settings` panel
    /// (oracle settingsService). Reads go through the in-memory snapshot;
    /// writes are write-through to Postgres.
    pub fn settings(&self) -> &db::SettingsStore {
        &self.settings
    }

    /// Synchronous snapshot read for panel rendering.
    pub fn settings_snapshot(&self) -> BotSettings {
        self.settings.get_settings()
    }

    /// Track cache repository (find-by-id, file-unique-id, delete).
    pub fn tracks(&self) -> &db::TracksRepository {
        &self.tracks
    }

    /// Apple Music catalog for track metadata lookups (`/info`).
    pub fn catalog(&self) -> &Catalog<ReqwestTransport> {
        &self.catalog
    }

    /// Apple Music playlist client (autodump's developer-token + catalog API).
    pub fn playlist(&self) -> &PlaylistClient<ReqwestPlaylistHttp> {
        &self.playlist
    }
}

impl OrchestratorDeps for RipDeps {
    async fn get_settings(&self) -> BotSettings {
        self.settings.get_settings()
    }

    async fn find_cached_tracks(
        &self,
        keys: &[TrackKey],
    ) -> Result<HashMap<TrackKey, CachedTrack>, String> {
        self.tracks
            .find_cached_tracks(keys)
            .await
            .map_err(|error| error.to_string())
    }

    async fn save_track(&self, input: SaveTrackInput) -> Result<(), String> {
        self.tracks
            .save_track(&input)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn delete_track(&self, track_key: &TrackKey) -> Result<bool, String> {
        self.tracks
            .delete_track(track_key)
            .await
            .map_err(|error| error.to_string())
    }

    async fn log_request(&self, log: RequestLog) -> Result<(), String> {
        self.requests
            .log_request(&log)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_album_tracks(&self, id: &str, storefront: &str) -> Result<AlbumTracks, String> {
        self.catalog
            .fetch_album_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_artist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<ArtistTracks, String> {
        self.catalog
            .fetch_artist_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_playlist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<PlaylistData, PlaylistError> {
        self.playlist.fetch_playlist_tracks(id, storefront).await
    }

    async fn rip(
        &self,
        track_id: &str,
        on_progress: Option<&RipProgressCallback>,
        storefront: &str,
        signal: CancellationToken,
        output_dir: Option<&Path>,
    ) -> Result<TrackRipResult, RipError> {
        self.ripper
            .rip(
                &self.ripper_deps,
                track_id,
                on_progress,
                storefront,
                Some(signal),
                output_dir,
            )
            .await
    }

    fn sink(&self) -> &dyn TelegramSink {
        &self.sink
    }

    fn upload_retry_base_ms(&self) -> u64 {
        self.upload_retry_base_ms
    }

    fn upload_max_retries(&self) -> u32 {
        self.max_retries
    }
}
