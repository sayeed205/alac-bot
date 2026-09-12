//! Filename policy for finalized ALAC tracks.

use crate::types::TrackMeta;

/// Replace filesystem-hostile characters with `_`; empty → `track`.
pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();
    let trimmed = sanitized.trim().to_owned();
    if trimmed.is_empty() {
        "track".to_owned()
    } else {
        trimmed
    }
}

/// Build the final output filename for a ripped track.
pub fn build_track_filename(meta: &TrackMeta) -> String {
    build_track_filename_with_codec(meta, "alac")
}

/// Same as [`build_track_filename`], labeled with the actually delivered
/// codec (`alac`, `mp4a.40.2`, `ec-3`).
pub fn build_track_filename_with_codec(meta: &TrackMeta, codec: &str) -> String {
    let number = meta.track_number.filter(|number| *number != 0).unwrap_or(1);
    let explicit = if meta.explicit { " [E]" } else { "" };
    let label = match codec {
        "ec-3" => "Atmos",
        "mp4a.40.2" | "mp4a.40.5" => "AAC",
        _ => "ALAC",
    };
    let combined = format!(
        "{number:02}. {} - {}{explicit} [{label}]",
        meta.title, meta.artist
    );
    format!("{}.m4a", sanitize_filename(&combined))
}
