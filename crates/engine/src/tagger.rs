//! Filename policy for finalized ALAC tracks.

use crate::types::TrackMeta;

/// Linux filesystems commonly cap one path component at 255 bytes. Keep the
/// limit in the provider-neutral filename seam so every local filename policy
/// uses the same byte-aware, UTF-8-safe bound.
pub const MAX_FILENAME_BYTES: usize = 255;

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

/// Truncate a filename component without splitting a UTF-8 code point.
pub fn bound_filename_component(name: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_owned();
    }
    let mut end = max_bytes.min(name.len());
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// Bound a sanitized filename while retaining a readable suffix such as a
/// codec label and extension. `suffix` must be the exact suffix of `name`.
pub fn bound_filename_with_suffix(name: &str, suffix: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_owned();
    }
    if suffix.len() > max_bytes || !name.ends_with(suffix) {
        return bound_filename_component(name, max_bytes);
    }
    let prefix = name.strip_suffix(suffix).unwrap_or(name);
    let prefix = bound_filename_component(prefix, max_bytes - suffix.len());
    format!("{prefix}{suffix}")
}

/// Build the final output filename for a ripped track.
pub fn build_track_filename(meta: &TrackMeta) -> String {
    build_track_filename_with_codec(meta, "alac")
}

/// Same as [`build_track_filename`], labeled with the actually delivered
/// codec (`alac`, `aac`, `mp4a.40.2`, `ec-3`).
pub fn build_track_filename_with_codec(meta: &TrackMeta, codec: &str) -> String {
    let number = meta.track_number.filter(|number| *number != 0).unwrap_or(1);
    let suffix = track_filename_suffix(meta.explicit, codec);
    let combined = format!("{number:02}. {} - {}{suffix}", meta.title, meta.artist);
    let sanitized = sanitize_filename(&combined);
    bound_filename_with_suffix(&sanitized, &suffix, MAX_FILENAME_BYTES)
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
