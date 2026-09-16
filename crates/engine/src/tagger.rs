//! Filename policy for finalized ALAC tracks.

use crate::filename::TrackFilename;
use crate::types::TrackMeta;

/// Build the final output filename for a ripped track.
pub fn build_track_filename(meta: &TrackMeta) -> TrackFilename {
    build_track_filename_with_codec(meta, "alac")
}

/// Same as [`build_track_filename`], labeled with the actually delivered
/// codec (`alac`, `aac`, `mp4a.40.2`, `ec-3`).
pub fn build_track_filename_with_codec(meta: &TrackMeta, codec: &str) -> TrackFilename {
    let number = meta.track_number.filter(|number| *number != 0).unwrap_or(1);
    let suffix = track_filename_suffix(meta.explicit, codec);
    let combined = format!("{number:02}. {} - {}{suffix}", meta.title, meta.artist);
    TrackFilename::sanitize_and_bound(&combined, Some(&suffix))
}

pub(crate) fn track_filename_suffix(explicit: bool, codec: &str) -> String {
    let explicit = if explicit { " [E]" } else { "" };
    let label = match codec {
        "ec-3" => "Atmos",
        "aac" | "mp4a.40.2" | "mp4a.40.5" => "AAC",
        _ => "ALAC",
    };
    format!("{explicit} [{label}].m4a")
}
