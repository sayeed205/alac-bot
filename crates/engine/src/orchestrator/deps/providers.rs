use std::future::Future;

use music::PlaylistData;

use crate::{
    ripper::{RipError, RipOptions},
    types::{AlbumTracks, ArtistTracks, Provider, TrackRipResult},
};

pub trait CollectionResolver: Send + Sync {
    fn fetch_album_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<AlbumTracks, String>> + Send;

    fn fetch_artist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<ArtistTracks, String>> + Send;

    fn fetch_playlist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<PlaylistData, String>> + Send;
}

pub trait TrackAcquisition: Send + Sync {
    fn rip(
        &self,
        track_id: &str,
        options: RipOptions<'_>,
    ) -> impl Future<Output = Result<TrackRipResult, RipError>> + Send;
}

pub trait ArtworkProvider: Send + Sync {
    fn fetch_artwork(&self, url: &str) -> impl Future<Output = Option<Vec<u8>>> + Send;
    fn artwork_url_at_size(&self, url: &str, size: u16) -> String;
}

pub trait ProviderPresentation: Send + Sync {
    fn default_job_header(&self) -> &str;
    fn album_url(&self, album_id: &str, storefront: &str) -> Option<String>;
    fn unavailable_track_message(&self) -> &str;
    fn unavailable_track_log_message(&self) -> &str;
}

pub trait ProviderComposition: Send + Sync + 'static {
    type Collections: CollectionResolver;
    type Acquisition: TrackAcquisition;
    type Artwork: ArtworkProvider;
    type Presentation: ProviderPresentation;

    fn provider(&self) -> Provider;
    fn collections(&self) -> &Self::Collections;
    fn acquisition(&self) -> &Self::Acquisition;
    fn artwork(&self) -> &Self::Artwork;
    fn presentation(&self) -> &Self::Presentation;
}

pub trait ProviderAccess: Send + Sync + 'static {
    type Providers: ProviderComposition;

    fn providers(&self) -> &Self::Providers;
}
