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
    types::{AlbumTracks, ArtistTracks, TrackRipResult},
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
        settings.init().await;

        let catalog = Catalog::new(ReqwestTransport::new());
        let ripper_catalog = Catalog::new(ReqwestTransport::new());
        let wrapper_url = env_option("ALAC_WRAPPER_URL");
        let wrapper_api_key = env_option("ALAC_WRAPPER_API_KEY");
        let stream_transport =
            engine::streaming::StreamTransport::new(engine::streaming::ReqwestHttp::new());
        let mirror_policy = engine::streaming::MirrorPolicyManager::new(
            engine::streaming::ReqwestHttp::new(),
            env_option("ALAC_MIRROR_URL").zip(env_option("ALAC_MIRROR_API_KEY")),
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
        let upload_retry_base_ms = std::env::var("ALAC_RETRY_BASE_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3000);

        Ok(Self {
            sink: FerogramTelegramSink::new(client, dump_peer).await?,
            tracks,
            requests,
            settings,
            catalog,
            playlist: PlaylistClient::new(ReqwestPlaylistHttp::new()),
            ripper: AlacTrackRipper::new(RipperConfig::default()),
            ripper_deps,
            mirror_policy: probe_policy,
            upload_retry_base_ms,
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
}

impl OrchestratorDeps for RipDeps {
    async fn get_settings(&self) -> BotSettings {
        self.settings.get_settings()
    }

    async fn find_cached_tracks(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, CachedTrack>, String> {
        self.tracks
            .find_cached_tracks(ids)
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

    async fn delete_track(&self, apple_track_id: &str) -> Result<bool, String> {
        self.tracks
            .delete_track(apple_track_id)
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
}
