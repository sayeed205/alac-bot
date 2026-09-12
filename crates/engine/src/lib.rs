//! Pure ALAC domain logic: URL parsing, iTunes catalog lookup, streaming,
//! tagging, lyrics. No Telegram, no database.

pub mod catalog;
pub mod limits;
pub mod lyrics;
pub mod orchestrator;
pub mod parser;
pub mod playlist;
pub mod progress;
pub mod queue;
pub mod ripper;
pub mod settings;
pub mod streaming;
pub mod tagger;
pub mod types;
pub mod wrapper;
pub mod zip;

pub use parser::{extract_batch_items, parse_alac_input, parse_single_item};
pub use types::{
    AlbumTracks, ArtistTracks, ChartAlbum, ParsedAlacInput, ParsedTargetItem, Provider, TargetKind,
    Codec, TrackKey, TrackMeta,
};
