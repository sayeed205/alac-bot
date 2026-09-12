//! Domain types shared across the core crate.
//!
//! These values flow into Telegram replies, DB rows, and media finalization,
//! so field shapes are part of the contract.

use diesel::{deserialize::FromSql, pg::Pg, serialize::ToSql};
use serde::{Deserialize, Serialize};

/// A catalog/cache provider supported by the bot.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    diesel::AsExpression,
    diesel::FromSqlRow,
)]
#[diesel(sql_type = diesel::sql_types::VarChar)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Apple,
    Spotify,
    Amazon,
}

impl Provider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Spotify => "spotify",
            Self::Amazon => "amazon",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "apple" => Ok(Self::Apple),
            "spotify" => Ok(Self::Spotify),
            "amazon" => Ok(Self::Amazon),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

impl ToSql<diesel::sql_types::VarChar, Pg> for Provider {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        <str as ToSql<diesel::sql_types::Text, Pg>>::to_sql(self.as_str(), out)
    }
}

impl FromSql<diesel::sql_types::VarChar, Pg> for Provider {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        <String as FromSql<diesel::sql_types::Text, Pg>>::from_sql(bytes)?
            .parse()
            .map_err(Into::into)
    }
}

/// An audio codec / encoding format supported by the bot.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    diesel::AsExpression,
    diesel::FromSqlRow,
    Default,
)]
#[diesel(sql_type = diesel::sql_types::VarChar)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    #[default]
    Alac,
    #[serde(rename = "ec-3")]
    Ec3,
    Aac,
    Flac,
}

impl Codec {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alac => "alac",
            Self::Ec3 => "ec-3",
            Self::Aac => "aac",
            Self::Flac => "flac",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Alac => "ALAC",
            Self::Ec3 => "Dolby Atmos",
            Self::Aac => "AAC",
            Self::Flac => "FLAC",
        }
    }
}

impl std::fmt::Display for Codec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Codec {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "alac" => Ok(Self::Alac),
            "ec-3" | "ec3" | "atmos" | "dolby" | "dolby atmos" | "eac3" => Ok(Self::Ec3),
            "aac" | "mp4a.40.2" | "mp4a.40.5" | "heaac" => Ok(Self::Aac),
            "flac" => Ok(Self::Flac),
            other => Err(format!("unknown codec: {other}")),
        }
    }
}

impl ToSql<diesel::sql_types::VarChar, Pg> for Codec {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        <str as ToSql<diesel::sql_types::Text, Pg>>::to_sql(self.as_str(), out)
    }
}

impl FromSql<diesel::sql_types::VarChar, Pg> for Codec {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        <String as FromSql<diesel::sql_types::Text, Pg>>::from_sql(bytes)?
            .parse()
            .map_err(Into::into)
    }
}

/// A globally unique track identity. The provider and optional codec are part of the identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackKey {
    pub provider: Provider,
    pub track_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<Codec>,
}

impl TrackKey {
    pub fn new(provider: Provider, track_id: impl Into<String>) -> Self {
        Self {
            provider,
            track_id: track_id.into(),
            codec: None,
        }
    }

    pub fn with_codec(mut self, codec: Codec) -> Self {
        self.codec = Some(codec);
        self
    }

    pub fn apple(track_id: impl Into<String>) -> Self {
        Self::new(Provider::Apple, track_id)
    }

    pub fn apple_codec(track_id: impl Into<String>, codec: Codec) -> Self {
        Self::new(Provider::Apple, track_id).with_codec(codec)
    }
}

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
    /// Explicit request to package an album as a ZIP (`-z`/`--zip`).
    pub zip: bool,
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
    /// First 10 chars of the raw release date, or `''` when absent
    /// (always a string, like `artwork_url`).
    pub release_date: String,
    /// Composer when the provider exposes it; otherwise omitted.
    pub composer: Option<String>,
    pub track_number: Option<i64>,
    pub track_count: Option<i64>,
    pub disc_number: Option<i64>,
    pub disc_count: Option<i64>,
    /// Duration in seconds (milliseconds / 1000, rounded).
    pub duration_secs: i64,
    pub explicit: bool,
    /// Provider advisory value (`explicit`, `clean`, or another inoffensive
    /// classification), retained so the media layer can write the precise
    /// iTunes rating atom.
    pub content_advisory: Option<String>,
    /// Missing artwork maps to `''` (never absent).
    pub artwork_url: String,
    /// Provider-native album and artist identifiers, when available.
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    /// Recording and release metadata exposed by the provider.
    pub isrc: Option<String>,
    pub record_label: Option<String>,
    pub copyright: Option<String>,
    pub upc: Option<String>,
    pub is_streamable: Option<bool>,
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

/// Result of a completed single-track rip.
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
