//! Provider-neutral ALAC domain logic: streaming, tagging, lyrics, and
//! orchestration. Provider implementations live in separate crates.

pub mod limits;
pub mod lyrics;
pub mod orchestrator;
pub mod progress;
pub mod queue;
pub mod ripper;
pub mod settings;
pub mod streaming;
pub mod tagger;
pub mod types;
pub mod zip;

pub use types::{
    AlbumTracks, ArtistTracks, Codec, ParsedAlacInput, ParsedTargetItem, Provider, TargetKind,
    TrackKey, TrackMeta,
};
