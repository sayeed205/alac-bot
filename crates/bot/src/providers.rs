//! Statically compiled provider composition for the bot.

use apple::{ApplePresentation, AppleProduction};
use engine::{
    orchestrator::deps::{
        ArtworkProvider, CollectionResolver, ProviderComposition, TrackAcquisition,
    },
    ripper::{AlacTrackRipper, RipError, RipperConfig},
    types::{AlbumTracks, ArtistTracks, Provider, TrackRipResult},
};
use music::PlaylistData;

/// The bot's provider registry is deliberately closed and compiled in. A job
/// selects one provider composition; there is no runtime plugin or provider
/// map to negotiate against.
pub struct ProviderRegistry {
    apple: AppleProduction,
    ripper: AlacTrackRipper,
}

impl ProviderRegistry {
    pub fn new(apple: AppleProduction, ripper_config: RipperConfig) -> Self {
        Self {
            apple,
            ripper: AlacTrackRipper::new(ripper_config),
        }
    }

    pub fn catalog(&self) -> &apple::Catalog<apple::ReqwestTransport> {
        self.apple.catalog()
    }

    pub fn playlist(&self) -> &apple::PlaylistClient<apple::ReqwestPlaylistHttp> {
        self.apple.playlist()
    }
}

impl CollectionResolver for ProviderRegistry {
    async fn fetch_album_tracks(&self, id: &str, storefront: &str) -> Result<AlbumTracks, String> {
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
        self.catalog()
            .fetch_artist_tracks(id, storefront)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_playlist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> Result<PlaylistData, String> {
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
        }
    }
}

impl ArtworkProvider for ProviderRegistry {
    async fn fetch_artwork(&self, url: &str) -> Option<Vec<u8>> {
        self.apple.ripper_deps().fetch_artwork_bytes(url).await
    }

    fn artwork_url_at_size(&self, url: &str, size: u16) -> String {
        apple::catalog::artwork_url_at_size(url, size)
    }
}

impl ProviderComposition for ProviderRegistry {
    type Collections = Self;
    type Acquisition = Self;
    type Artwork = Self;
    type Presentation = ApplePresentation;

    fn provider(&self) -> Provider {
        Provider::Apple
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
        static PRESENTATION: ApplePresentation = ApplePresentation;
        &PRESENTATION
    }
}
