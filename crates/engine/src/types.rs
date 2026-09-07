//! Domain types shared across the core crate.
//!
//! Field-level parity with the TS oracle (`src/modules/alac/types.ts`) matters:
//! these values flow into Telegram replies, DB rows, and ffmpeg tags.

/// The kind of Apple Music target a parsed link/id refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetKind {
    Track,
    Album,
    Playlist,
    Artist,
}

/// One parsed link/id token, e.g. `?i=` track, album, `pl.` playlist, artist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedTargetItem {
    pub id: String,
    pub kind: TargetKind,
    /// Lowercased two-letter storefront captured from the URL, when present.
    pub storefront: Option<String>,
}

/// Full result of parsing a user command (or batch file line set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlacInput {
    pub items: Vec<ParsedTargetItem>,
    pub track_id: String,
    pub force: bool,
    pub is_album: bool,
    pub is_playlist: bool,
    pub is_artist: bool,
    pub storefront: Option<String>,
}

/// Apple Music track metadata resolved from the iTunes catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMeta {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: Option<String>,
    /// First 10 chars of the raw release date, or `''` when absent — the TS
    /// oracle always produces a string here (like `artwork_url`).
    pub release_date: String,
    /// Never set by the catalog (TS leaves it undefined; the tagger fills it).
    pub composer: Option<String>,
    pub track_number: Option<i64>,
    pub track_count: Option<i64>,
    pub disc_number: Option<i64>,
    pub disc_count: Option<i64>,
    /// Duration in seconds (milliseconds / 1000, rounded).
    pub duration_secs: i64,
    pub explicit: bool,
    /// TS maps missing artwork to `''` (not undefined) — kept for parity.
    pub artwork_url: String,
}

/// An album plus its ordered track list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTracks {
    pub album: TrackMeta,
    pub tracks: Vec<TrackMeta>,
}

/// An artist's resolved discography tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistTracks {
    pub artist_id: String,
    pub artist_name: String,
    pub tracks: Vec<TrackMeta>,
}

/// Result of a completed single-track rip (parity with TS TrackRipResult).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRipResult {
    pub file_path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i64,
    pub bit_depth: u32,
    pub sample_rate: u32,
    pub codec: String,
    pub genre: String,
    pub release_date: String,
    pub track_number: i64,
    pub track_count: i64,
}

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
