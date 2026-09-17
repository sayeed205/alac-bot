//! Statically compiled provider composition for the bot.

use apple::{ApplePresentation, AppleProduction};
use engine::{
    orchestrator::deps::{
        ArtworkProvider, CollectionResolver, ProviderComposition, ProviderPresentation,
        TrackAcquisition,
    },
    ripper::{AlacTrackRipper, RipError, RipperConfig},
    types::{AlbumTracks, ArtistTracks, Provider, TrackRipResult},
};
use music::PlaylistData;

/// Combined presentation handler for all supported providers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderRegistryPresentation {
    apple: ApplePresentation,
    qobuz: qobuz::QobuzPresentation,
}

impl ProviderRegistryPresentation {
    pub const fn new() -> Self {
        Self {
            apple: ApplePresentation,
            qobuz: qobuz::QobuzPresentation,
        }
    }
}

impl ProviderPresentation for ProviderRegistryPresentation {
    fn default_job_header(&self) -> &str {
        "ALAC Lossless Rip"
    }

    fn album_url(&self, album_id: &str, storefront: &str) -> Option<String> {
        if storefront == "qobuz" {
            self.qobuz.album_url(album_id, storefront)
        } else {
            self.apple.album_url(album_id, storefront)
        }
    }

    fn unavailable_track_message(&self) -> &str {
        "Unavailable on music provider (not streamable or georestricted)"
    }

    fn unavailable_track_log_message(&self) -> &str {
        "Track is not streamable in provider catalog, skipping rip"
    }
}

/// The bot's provider registry is deliberately closed and compiled in. A job
/// selects one provider composition; there is no runtime plugin or provider
/// map to negotiate against.
pub struct ProviderRegistry {
    apple: AppleProduction,
    qobuz: Option<qobuz::QobuzProduction>,
    ripper: AlacTrackRipper,
}

impl ProviderRegistry {
    pub fn new(
        apple: AppleProduction,
        qobuz: Option<qobuz::QobuzProduction>,
        ripper_config: RipperConfig,
    ) -> Self {
        Self {
            apple,
            qobuz,
            ripper: AlacTrackRipper::new(ripper_config),
        }
    }

    pub fn catalog(&self) -> &apple::Catalog<apple::ReqwestTransport> {
        self.apple.catalog()
    }

    pub fn playlist(&self) -> &apple::PlaylistClient<apple::ReqwestPlaylistHttp> {
        self.apple.playlist()
    }

    pub fn qobuz(&self) -> Option<&qobuz::QobuzProduction> {
        self.qobuz.as_ref()
    }
}

impl CollectionResolver for ProviderRegistry {
    async fn fetch_album_tracks(&self, id: &str, storefront: &str) -> Result<AlbumTracks, String> {
        if storefront == "qobuz" {
            if let Some(qobuz) = &self.qobuz {
                return qobuz.catalog().fetch_album_tracks(id, storefront).await;
            } else {
                return Err("Qobuz provider is not configured".to_string());
            }
        }
        self.catalog()
            .fetch_album_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_artist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<ArtistTracks, String> {
        if storefront == "qobuz" {
            if let Some(qobuz) = &self.qobuz {
                return qobuz.catalog().fetch_artist_tracks(id, storefront).await;
            } else {
                return Err("Qobuz provider is not configured".to_string());
            }
        }
        self.catalog()
            .fetch_artist_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_artist_album_ids(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<Vec<String>, String> {
        if storefront == "qobuz" {
            if let Some(qobuz) = &self.qobuz {
                return qobuz.catalog().fetch_artist_album_ids(id, storefront).await;
            } else {
                return Err("Qobuz provider is not configured".to_string());
            }
        }
        self.catalog()
            .fetch_artist_album_ids(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_playlist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<PlaylistData, String> {
        if storefront == "qobuz" {
            if let Some(qobuz) = &self.qobuz {
                return qobuz.catalog().fetch_playlist_tracks(id, storefront).await;
            } else {
                return Err("Qobuz provider is not configured".to_string());
            }
        }
        self.playlist()
            .fetch_playlist_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }
}

impl TrackAcquisition for ProviderRegistry {
    async fn rip(
        &self,
        track_id: &str,
        options: engine::ripper::RipOptions<'_>,
    ) -> Result<TrackRipResult, RipError> {
        match options.provider {
            Provider::Apple => {
                self.ripper
                    .rip(self.apple.ripper_deps(), track_id, options)
                    .await
            }
            Provider::Qobuz => {
                if let Some(qobuz) = &self.qobuz {
                    self.ripper
                        .rip(qobuz.acquisition(), track_id, options)
                        .await
                } else {
                    Err(RipError::TrackUnavailable {
                        reason: "Qobuz provider is not configured".to_string(),
                    })
                }
            }
        }
    }
}

impl ArtworkProvider for ProviderRegistry {
    async fn fetch_artwork(&self, url: &str) -> Option<Vec<u8>> {
        engine::ripper::fetch_artwork_bytes(self.ripper.config(), url).await
    }

    fn artwork_url_at_size(&self, url: &str, size: u16) -> String {
        if url.contains("static.qobuz.com") || url.contains("qobuz") {
            url.to_string()
        } else {
            apple::catalog::artwork_url_at_size(url, size)
        }
    }
}

impl ProviderComposition for ProviderRegistry {
    type Collections = Self;
    type Acquisition = Self;
    type Artwork = Self;
    type Presentation = ProviderRegistryPresentation;

    fn provider(&self) -> Provider {
        Provider::Apple
    }

    fn supports_provider(&self, provider: Provider) -> bool {
        match provider {
            Provider::Apple => true,
            Provider::Qobuz => self.qobuz.is_some(),
        }
    }

    fn collections(&self) -> &Self::Collections {
        self
    }

    fn acquisition(&self) -> &Self::Acquisition {
        self
    }

    fn artwork(&self) -> &Self::Artwork {
        self
    }

    fn presentation(&self) -> &Self::Presentation {
        static PRESENTATION: ProviderRegistryPresentation = ProviderRegistryPresentation::new();
        &PRESENTATION
    }
}
