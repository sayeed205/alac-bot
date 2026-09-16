//! Data types and Serde models for Qobuz API responses and domain mapping.

use music::{AlbumTracks, PlaylistData, PlaylistTrack, TrackMeta};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzImage {
    pub small: Option<String>,
    pub thumbnail: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzArtistRef {
    pub id: Option<u64>,
    pub name: String,
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzGenre {
    pub id: Option<u64>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzLabel {
    pub id: Option<u64>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzAlbum {
    pub id: serde_json::Value,
    pub title: String,
    pub artist: Option<QobuzArtistRef>,
    pub artists: Option<Vec<QobuzArtistRef>>,
    pub image: Option<QobuzImage>,
    pub duration: Option<i64>,
    pub tracks_count: Option<i64>,
    pub genre: Option<QobuzGenre>,
    pub label: Option<QobuzLabel>,
    pub release_date_original: Option<String>,
    pub release_date_download: Option<String>,
    pub release_date_stream: Option<String>,
    pub upc: Option<String>,
    pub copyright: Option<String>,
    pub maximum_bit_depth: Option<u32>,
    pub maximum_sampling_rate: Option<f64>,
    pub tracks: Option<QobuzTrackList>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzTrackList {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub total: Option<i64>,
    pub items: Vec<QobuzTrack>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzTrack {
    pub id: serde_json::Value,
    pub title: String,
    pub version: Option<String>,
    pub duration: Option<i64>,
    pub track_number: Option<i64>,
    pub media_number: Option<i64>,
    pub performers: Option<String>,
    pub performer: Option<QobuzArtistRef>,
    pub composer: Option<QobuzArtistRef>,
    pub album: Option<Box<QobuzAlbum>>,
    pub isrc: Option<String>,
    pub copyright: Option<String>,
    pub maximum_bit_depth: Option<u32>,
    pub maximum_sampling_rate: Option<f64>,
    pub parental_warning: Option<bool>,
    pub streamable: Option<bool>,
    pub release_date_original: Option<String>,
    pub release_date_stream: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzPlaylist {
    pub id: serde_json::Value,
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub owner: Option<QobuzOwner>,
    pub tracks_count: Option<i64>,
    pub tracks: Option<QobuzTrackList>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzOwner {
    pub id: Option<u64>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzStreamData {
    pub track_id: Option<serde_json::Value>,
    pub url: Option<String>,
    pub format_id: Option<u32>,
    pub mime_type: Option<String>,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<f64>,
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzStreamUrlResponse {
    pub success: bool,
    #[serde(rename = "trackId")]
    pub track_id: Option<serde_json::Value>,
    pub url: Option<String>,
    pub data: Option<QobuzStreamData>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzTrackResponse {
    pub success: bool,
    pub track: Option<QobuzTrack>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzAlbumResponse {
    pub success: bool,
    pub album: Option<QobuzAlbum>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzPlaylistResponse {
    pub success: bool,
    pub playlist: Option<QobuzPlaylist>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzArtistResponse {
    pub success: bool,
    pub artist: Option<QobuzArtistData>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzArtistData {
    pub id: Option<u64>,
    pub name: String,
    pub albums: Option<QobuzAlbumList>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QobuzAlbumList {
    pub items: Vec<QobuzAlbum>,
}

impl QobuzTrack {
    pub fn id_string(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    pub fn to_track_meta(&self, fallback_album: Option<&QobuzAlbum>) -> TrackMeta {
        let album_ref = self.album.as_deref().or(fallback_album);
        let album_title = album_ref.map_or("", |a| a.title.as_str()).to_owned();
        let album_artist = album_ref
            .and_then(|a| a.artist.as_ref().map(|ar| ar.name.clone()))
            .unwrap_or_else(|| {
                self.performer
                    .as_ref()
                    .map_or("Unknown Artist", |p| p.name.as_str())
                    .to_owned()
            });

        let artist_name = self
            .performer
            .as_ref()
            .map_or_else(|| album_artist.clone(), |p| p.name.clone());

        let artwork_url = album_ref
            .and_then(|a| a.image.as_ref())
            .and_then(|img| img.large.as_ref().or(img.small.as_ref()))
            .cloned()
            .unwrap_or_default();

        let release_date = self
            .release_date_original
            .as_ref()
            .or(self.release_date_stream.as_ref())
            .or_else(|| album_ref.and_then(|a| a.release_date_original.as_ref()))
            .map_or(String::new(), |d| d.chars().take(10).collect());

        let genre = album_ref.and_then(|a| a.genre.as_ref().map(|g| g.name.clone()));
        let label = album_ref.and_then(|a| a.label.as_ref().map(|l| l.name.clone()));
        let copyright = self
            .copyright
            .clone()
            .or_else(|| album_ref.and_then(|a| a.copyright.clone()));

        let album_id = album_ref.map(|a| match &a.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });

        let artist_id = self
            .performer
            .as_ref()
            .and_then(|p| p.id.map(|id| id.to_string()))
            .or_else(|| {
                album_ref.and_then(|a| {
                    a.artist
                        .as_ref()
                        .and_then(|ar| ar.id.map(|id| id.to_string()))
                })
            });

        TrackMeta {
            id: self.id_string(),
            title: self.title.clone(),
            artist: artist_name,
            album: album_title,
            album_artist,
            genre,
            release_date,
            composer: self.composer.as_ref().map(|c| c.name.clone()),
            track_number: self.track_number,
            track_count: album_ref.and_then(|a| a.tracks_count),
            disc_number: self.media_number,
            disc_count: None,
            duration_secs: self.duration.unwrap_or(0),
            explicit: self.parental_warning.unwrap_or(false),
            content_advisory: self.parental_warning.map(|w| {
                if w {
                    "explicit".to_owned()
                } else {
                    "clean".to_owned()
                }
            }),
            artwork_url,
            album_id,
            artist_id,
            isrc: self.isrc.clone(),
            record_label: label,
            copyright,
            upc: album_ref.and_then(|a| a.upc.clone()),
            is_streamable: self.streamable,
        }
    }
}

impl QobuzAlbum {
    pub fn id_string(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    pub fn to_album_tracks(&self) -> AlbumTracks {
        let dummy_track = QobuzTrack {
            id: self.id.clone(),
            title: self.title.clone(),
            version: None,
            duration: self.duration,
            track_number: None,
            media_number: None,
            performers: None,
            performer: self.artist.clone(),
            composer: None,
            album: None,
            isrc: None,
            copyright: self.copyright.clone(),
            maximum_bit_depth: self.maximum_bit_depth,
            maximum_sampling_rate: self.maximum_sampling_rate,
            parental_warning: None,
            streamable: Some(true),
            release_date_original: self.release_date_original.clone(),
            release_date_stream: self.release_date_stream.clone(),
        };

        let album_meta = dummy_track.to_track_meta(Some(self));
        let tracks = self
            .tracks
            .as_ref()
            .map(|tl| {
                tl.items
                    .iter()
                    .map(|t| t.to_track_meta(Some(self)))
                    .collect()
            })
            .unwrap_or_default();

        AlbumTracks {
            album: album_meta,
            tracks,
        }
    }
}

impl QobuzPlaylist {
    pub fn id_string(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    pub fn to_playlist_data(&self) -> PlaylistData {
        let name = self
            .name
            .clone()
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| "Qobuz Playlist".to_owned());

        let tracks = self
            .tracks
            .as_ref()
            .map(|tl| {
                tl.items
                    .iter()
                    .map(|t| PlaylistTrack {
                        id: t.id_string(),
                        title: t.title.clone(),
                        artist: t
                            .performer
                            .as_ref()
                            .map_or("Unknown Artist", |p| p.name.as_str())
                            .to_owned(),
                        duration: t.duration.and_then(|d| u64::try_from(d).ok()),
                    })
                    .collect()
            })
            .unwrap_or_default();

        PlaylistData {
            id: self.id_string(),
            title: name,
            curator_name: self.owner.as_ref().map(|o| o.name.clone()),
            description: self.description.clone(),
            tracks,
        }
    }
}
