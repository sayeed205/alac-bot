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
    let number = meta.track_number.filter(|number| *number != 0).unwrap_or(1);
    let explicit = if meta.explicit { " [E]" } else { "" };
    let combined = format!(
        "{number:02}. {} - {}{explicit} [ALAC]",
        meta.title, meta.artist
    );
    format!("{}.m4a", sanitize_filename(&combined))
}
