//! Engine-facing type compatibility shims.
//!
//! Shared music domain types live in [`music`]. They are re-exported here so
//! existing engine and bot callers retain their public paths.

pub use music::{
    AlbumTracks, ArtistTracks, Codec, ParsedAlacInput, ParsedTargetItem, Provider, TargetKind,
    TrackKey, TrackMeta, TrackRipResult,
};

/// A charts album entry from the Apple RSS feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub url: String,
    pub artwork_url: Option<String>,
    pub release_date: Option<String>,
    pub genre: Option<String>,
}
