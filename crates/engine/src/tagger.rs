//! Filename policy for finalized audio tracks.

use crate::{filename::TrackFilename, types::TrackMeta};

/// Build the final output filename for a ripped track.
pub fn build_track_filename(meta: &TrackMeta) -> TrackFilename {
    build_track_filename_with_codec(meta, "alac")
}

/// Same as [`build_track_filename`], labeled with the actually delivered
/// codec (`alac`, `aac`, `mp4a.40.2`, `ec-3`, `flac`, `mp3`).
pub fn build_track_filename_with_codec(meta: &TrackMeta, codec: &str) -> TrackFilename {
    let number = meta.track_number.filter(|number| *number != 0).unwrap_or(1);
    let suffix = track_filename_suffix(meta.explicit, codec);
    let combined = format!("{number:02}. {} - {}{suffix}", meta.title, meta.artist);
    TrackFilename::sanitize_and_bound(&combined, Some(&suffix))
}

pub(crate) fn track_filename_suffix(explicit: bool, codec: &str) -> String {
    let explicit = if explicit { " [E]" } else { "" };
    let (label, ext) = match codec {
        "ec-3" => ("Atmos", "m4a"),
        "aac" | "mp4a.40.2" | "mp4a.40.5" => ("AAC", "m4a"),
        "flac" => ("FLAC", "flac"),
        "mp3" => ("MP3", "mp3"),
        _ => ("ALAC", "m4a"),
    };
    format!("{explicit} [{label}].{ext}")
}
