//! Engine-facing type compatibility shims.
//!
//! Shared music domain types live in [`music`]. They are re-exported here so
//! existing engine and bot callers retain their public paths.

pub use music::{
    AlbumTracks, ArtistTracks, Codec, ParsedAlacInput, ParsedTargetItem, Provider, TargetKind,
    TrackKey, TrackMeta, TrackRipResult,
};
