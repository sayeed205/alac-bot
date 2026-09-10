//! Dump-channel caption formatting and parsing.
//!
//! The caption has two audiences: a compact operator-facing summary and a
//! canonical machine-readable JSON record consumed by the indexer. Telegram's
//! HTML parser is the equivalent here — the bot crate feeds the returned HTML
//! string to ferogram's HTML support. Captions are clean-slate: the indexer
//! accepts only the current payload shape and does not support older formats.

use crate::types::{Provider, TrackKey, TrackRipResult};

/// TS `DumpCaptionMetadata` — everything optional the TS type marks optional,
/// everything else required.
#[derive(Debug, Clone, PartialEq)]
pub struct DumpCaptionMetadata<'a> {
    pub track_key: TrackKey,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration: i64,
    pub bit_depth: u32,
    pub sample_rate: u32,
    pub codec: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub release_date: Option<&'a str>,
    pub track_number: Option<i64>,
    pub track_count: Option<i64>,
}

impl<'a> From<(&'a TrackRipResult, &'a str)> for DumpCaptionMetadata<'a> {
    fn from((rip, track_id): (&'a TrackRipResult, &'a str)) -> Self {
        DumpCaptionMetadata {
            track_key: TrackKey::new(Provider::Apple, track_id),
            title: &rip.title,
            artist: &rip.artist,
            album: &rip.album,
            duration: rip.duration,
            bit_depth: rip.bit_depth,
            sample_rate: rip.sample_rate,
            codec: Some(&rip.codec),
            genre: Some(&rip.genre),
            release_date: Some(&rip.release_date),
            track_number: Some(rip.track_number),
            track_count: Some(rip.track_count),
        }
    }
}

/// Machine-readable metadata for formatting an album ZIP caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpZipCaptionMetadata<'a> {
    pub provider: Provider,
    pub album_id: &'a str,
    pub album: &'a str,
    pub artist: &'a str,
    pub filename: &'a str,
    pub part_index: i32,
    pub total_parts: i32,
    pub generation_hash: &'a str,
}

/// Metadata used to format the rich album details caption (delivered alongside the preview photo or as fallback text).
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumDetailsCaptionMetadata<'a> {
    pub album: &'a str,
    pub artist: &'a str,
    pub album_id: &'a str,
    pub storefront: &'a str,
    pub total_tracks: usize,
    pub delivered_tracks: usize,
    pub size_bytes: i64,
    pub total_parts: usize,
    pub release_year: &'a str,
    pub genre: Option<&'a str>,
    pub record_label: Option<&'a str>,
    pub is_partial: bool,
    pub user_name: Option<&'a str>,
    pub user_id: i64,
}

/// Parsed metadata extracted from an album ZIP dump caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedZipDumpMetadata {
    pub provider: Provider,
    pub album_id: String,
    pub album: String,
    pub artist: String,
    pub part_index: i32,
    pub total_parts: i32,
    pub generation_hash: String,
}

/// mtcute `html.escape` parity: `& < > " '` (hex entities).
pub fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Formats a user mention with clickable link (Telegram handle or tg:// user link).
pub fn format_requester_mention(user_name: Option<&str>, user_id: i64) -> String {
    let name = user_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("User");
    if let Some(handle) = name.strip_prefix('@') {
        format!(r#"<a href="https://t.me/{handle}">@{handle}</a>"#)
    } else if user_id > 0 {
        format!(r#"<a href="tg://user?id={user_id}">{}</a>"#, html_escape(name))
    } else {
        html_escape(name)
    }
}

/// Formats the rich album details caption (displayed with album preview photo or fallback text).
pub fn format_album_details_caption(meta: &AlbumDetailsCaptionMetadata<'_>) -> String {
    let album_link = if !meta.album_id.is_empty() {
        let sf = if meta.storefront.is_empty() {
            "us"
        } else {
            meta.storefront
        };
        format!(
            r#"💿 <a href="https://music.apple.com/{sf}/album/{}"><b>{}</b></a>"#,
            meta.album_id,
            html_escape(meta.album)
        )
    } else {
        format!("💿 <b>{}</b>", html_escape(meta.album))
    };

    let tracks = if meta.is_partial {
        format!("{}/{} tracks", meta.delivered_tracks, meta.total_tracks)
    } else {
        format!("{} tracks", meta.delivered_tracks)
    };

    let size = crate::progress::format_bytes(meta.size_bytes.max(0) as u64);
    let parts_info = if meta.total_parts > 1 {
        format!(" · {} parts", meta.total_parts)
    } else {
        String::new()
    };

    let mention = format_requester_mention(meta.user_name, meta.user_id);

    let mut bullets = Vec::new();
    bullets.push(format!("• <b>Tracks:</b> {tracks}"));
    bullets.push(format!("• <b>Size:</b> {size}{parts_info}"));
    if !meta.release_year.is_empty() {
        bullets.push(format!("• <b>Released:</b> {}", html_escape(meta.release_year)));
    }
    if let Some(genre) = meta.genre.filter(|s| !s.is_empty()) {
        bullets.push(format!("• <b>Genre:</b> {}", html_escape(genre)));
    }
    if let Some(label) = meta.record_label.filter(|s| !s.is_empty()) {
        bullets.push(format!("• <b>Label:</b> {}", html_escape(label)));
    }
    bullets.push("• <b>Quality:</b> Lossless · ALAC".to_string());
    if meta.is_partial {
        bullets.push("• ⚠️ <b>Note:</b> Partial archive".to_string());
    }
    bullets.push(format!("• <b>Requested by:</b> {mention}"));

    format!(
        "{album_link}<br/>👤 <b>Artist:</b> {}<br/><br/><blockquote>{}</blockquote>",
        html_escape(meta.artist),
        bullets.join("<br/>")
    )
}

/// Formats a compact operational caption.
///
/// The first three lines are for quick scanning in the dump channel. The
/// expandable block contains the single canonical machine record consumed by
/// the indexer; human-readable metadata is intentionally not repeated there.
pub fn format_dump_caption(meta: &DumpCaptionMetadata<'_>) -> String {
    let m = meta.duration / 60;
    let s = format!("{:02}", meta.duration % 60);
    let track_number = meta.track_number.unwrap_or(1);
    let track_count = meta.track_count.unwrap_or(1);
    let summary = format!(
        "<b>{}</b> — {}<br/><i>{}</i> · <code>{track_number}/{track_count}</code><br/><code>ALAC · {}-bit · {:.1} kHz · {m}:{s}</code>",
        html_escape(meta.title),
        html_escape(meta.artist),
        html_escape(meta.album),
        meta.bit_depth,
        meta.sample_rate as f64 / 1000.0,
    );

    // Machine payload. TS: JSON.stringify(payload, null, 2), each line
    // escaped, joined with <br/>.
    let payload_json = serde_json::json!({
        "provider": meta.track_key.provider,
        "track_id": meta.track_key.track_id,
        "title": meta.title,
        "artist": meta.artist,
        "album": meta.album,
        "dur": meta.duration,
        "bit": meta.bit_depth,
        "hz": meta.sample_rate,
        "genre": meta.genre.unwrap_or("Music"),
        "date": meta.release_date.unwrap_or(""),
        "trk": meta.track_number.unwrap_or(1),
        "cnt": meta.track_count.unwrap_or(1),
    });
    let pretty = serde_json::to_string_pretty(&payload_json).expect("payload serializes");
    let payload_html = pretty
        .lines()
        .map(html_escape)
        .collect::<Vec<_>>()
        .join("<br/>");

    format!("{summary}<br/><blockquote expandable>{payload_html}</blockquote>")
}

/// Formats an album ZIP caption.
///
/// If `total_parts <= 1`, the "part 1 of 1" line is omitted entirely.
/// If `total_parts > 1`, "Album ZIP part X of Y" is included.
/// Complete archives include an expandable blockquote with the machine-readable JSON payload.
pub fn format_zip_dump_caption(
    meta: &DumpZipCaptionMetadata<'_>,
    is_complete: bool,
    failed_count: usize,
) -> String {
    let header = format!("<b>{}</b>", html_escape(meta.filename));
    let part_line = if meta.total_parts > 1 {
        format!(
            "<br/><i>Album ZIP part {} of {}</i>",
            meta.part_index, meta.total_parts
        )
    } else {
        String::new()
    };
    let partial_note = if is_complete {
        String::new()
    } else {
        format!("<br/>Partial archive; {failed_count} track(s) failed.")
    };
    let summary = format!("{header}{part_line}{partial_note}");

    if !is_complete {
        return summary;
    }

    let payload_json = serde_json::json!({
        "type": "album_zip",
        "provider": meta.provider,
        "album_id": meta.album_id,
        "album": meta.album,
        "artist": meta.artist,
        "part": meta.part_index,
        "total_parts": meta.total_parts,
        "hash": meta.generation_hash,
    });
    let pretty = serde_json::to_string_pretty(&payload_json).expect("payload serializes");
    let payload_html = pretty
        .lines()
        .map(html_escape)
        .collect::<Vec<_>>()
        .join("<br/>");

    format!("{summary}<br/><blockquote expandable>{payload_html}</blockquote>")
}

/// TS `ParsedDumpMetadata` — the round-trip shape of the embedded payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDumpMetadata {
    pub track_key: TrackKey,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i64,
    pub bit_depth: u32,
    pub sample_rate: u32,
    pub genre: String,
    pub release_date: String,
    pub track_number: i64,
    pub track_count: i64,
}

/// Extracts structured track metadata from a message caption. Returns
/// `None` for missing, stale, unrelated, or invalid records.
pub fn parse_dump_caption(text: Option<&str>) -> Option<ParsedDumpMetadata> {
    let text = text?;
    let payload = extract_payload(text)?;
    let parsed: serde_json::Value = serde_json::from_str(&payload).ok()?;
    let provider = parsed.get("provider")?.as_str()?.parse::<Provider>().ok()?;
    let track_id = parsed.get("track_id")?.as_str()?;
    if track_id.is_empty() {
        return None;
    }

    Some(ParsedDumpMetadata {
        track_key: TrackKey::new(provider, track_id),
        title: string_field(&parsed, "title"),
        artist: string_field(&parsed, "artist"),
        album: string_field(&parsed, "album"),
        duration: number_field(&parsed, "dur", 0),
        bit_depth: number_field(&parsed, "bit", 16).max(0) as u32,
        sample_rate: number_field(&parsed, "hz", 44100).max(0) as u32,
        genre: string_field_or(&parsed, "genre", "Music"),
        release_date: string_field(&parsed, "date"),
        track_number: number_field(&parsed, "trk", 1),
        track_count: number_field(&parsed, "cnt", 1),
    })
}

/// Extracts structured album ZIP metadata from a message caption.
pub fn parse_zip_dump_caption(text: Option<&str>) -> Option<ParsedZipDumpMetadata> {
    let text = text?;
    let payload = extract_zip_payload(text)?;
    let parsed: serde_json::Value = serde_json::from_str(&payload).ok()?;
    let provider = parsed.get("provider")?.as_str()?.parse::<Provider>().ok()?;
    let album_id = parsed.get("album_id")?.as_str()?;
    if album_id.is_empty() {
        return None;
    }

    Some(ParsedZipDumpMetadata {
        provider,
        album_id: album_id.to_owned(),
        album: string_field(&parsed, "album"),
        artist: string_field(&parsed, "artist"),
        part_index: number_field(&parsed, "part", 1).max(1) as i32,
        total_parts: number_field(&parsed, "total_parts", 1).max(1) as i32,
        generation_hash: string_field(&parsed, "hash"),
    })
}

fn string_field(value: &serde_json::Value, key: &str) -> String {
    string_field_or(value, key, "")
}

fn string_field_or(value: &serde_json::Value, key: &str, default: &str) -> String {
    match value.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => default.to_string(),
    }
}

fn number_field(value: &serde_json::Value, key: &str, default: i64) -> i64 {
    match value.get(key) {
        Some(v) => v.as_i64().unwrap_or(default),
        None => default,
    }
}

fn extract_payload(text: &str) -> Option<String> {
    extract_balanced_json(text, "\"track_id\"")
}

fn extract_zip_payload(text: &str) -> Option<String> {
    extract_balanced_json(text, "\"album_id\"")
}

fn extract_balanced_json(text: &str, required_key: &str) -> Option<String> {
    let normalized = text
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");

    for (start, _) in normalized.match_indices('{') {
        let candidate = &normalized[start..];
        if !candidate.contains(required_key) {
            continue;
        }
        let mut depth = 0;
        let mut in_str = false;
        let mut escape = false;
        let mut end_idx = None;

        for (idx, ch) in candidate.char_indices() {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' && in_str {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_str = !in_str;
                continue;
            }
            if !in_str {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        end_idx = Some(idx);
                        break;
                    }
                }
            }
        }

        if let Some(end) = end_idx {
            let json_str = &candidate[..=end];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let stripped_key = required_key.trim_matches('"');
                if parsed.get(stripped_key).is_some() {
                    return Some(json_str.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> DumpCaptionMetadata<'static> {
        DumpCaptionMetadata {
            track_key: TrackKey::apple("1440828878"),
            title: "Night Song",
            artist: "A&R <duo>",
            album: "Escapes",
            duration: 215,
            bit_depth: 24,
            sample_rate: 48000,
            codec: Some("alac"),
            genre: Some("Electronic"),
            release_date: Some("2021-06-04"),
            track_number: Some(2),
            track_count: Some(10),
        }
    }

    #[test]
    fn format_album_details_caption_single_part() {
        let meta = AlbumDetailsCaptionMetadata {
            album: "Fossils, Vol. 1",
            artist: "Rupam Islam & Fossils",
            album_id: "1440828878",
            storefront: "in",
            total_tracks: 8,
            delivered_tracks: 8,
            size_bytes: 289_950_924,
            total_parts: 1,
            release_year: "2001",
            genre: Some("Rock"),
            record_label: Some("Asha Audio"),
            is_partial: false,
            user_name: Some("@sayeed69"),
            user_id: 123456,
        };
        let html = format_album_details_caption(&meta);
        assert!(html.contains(r#"💿 <a href="https://music.apple.com/in/album/1440828878"><b>Fossils, Vol. 1</b></a>"#));
        assert!(html.contains("👤 <b>Artist:</b> Rupam Islam &amp; Fossils"));
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("• <b>Tracks:</b> 8 tracks"));
        assert!(html.contains("• <b>Size:</b> 276.52MB"));
        // Single part: NO "parts" text
        assert!(!html.contains("parts"));
        assert!(html.contains("• <b>Released:</b> 2001"));
        assert!(html.contains("• <b>Genre:</b> Rock"));
        assert!(html.contains("• <b>Label:</b> Asha Audio"));
        assert!(html.contains("• <b>Quality:</b> Lossless · ALAC"));
        assert!(html.contains(r#"• <b>Requested by:</b> <a href="https://t.me/sayeed69">@sayeed69</a>"#));
        assert!(html.ends_with("</blockquote>"));
    }

    #[test]
    fn format_album_details_caption_multi_part_partial() {
        let meta = AlbumDetailsCaptionMetadata {
            album: "Greatest Hits",
            artist: "Queen",
            album_id: "987654321",
            storefront: "us",
            total_tracks: 17,
            delivered_tracks: 15,
            size_bytes: 4_294_967_296,
            total_parts: 3,
            release_year: "1981",
            genre: None,
            record_label: None,
            is_partial: true,
            user_name: Some("John Doe"),
            user_id: 78910,
        };
        let html = format_album_details_caption(&meta);
        assert!(html.contains("• <b>Tracks:</b> 15/17 tracks"));
        assert!(html.contains(" · 3 parts"));
        assert!(html.contains("• ⚠️ <b>Note:</b> Partial archive"));
        assert!(html.contains(r#"• <b>Requested by:</b> <a href="tg://user?id=78910">John Doe</a>"#));
    }

    #[test]
    fn caption_is_compact_and_keeps_machine_record() {
        let html = format_dump_caption(&sample_meta());
        assert!(html.starts_with("<b>Night Song</b> — A&amp;R &lt;duo&gt;"));
        assert!(html.contains("<i>Escapes</i> · <code>2/10</code>"));
        assert!(html.contains("<code>ALAC · 24-bit · 48.0 kHz · 3:35</code>"));
        assert!(html.contains("<blockquote expandable>"));
        // Payload JSON is escaped and <br/>-joined.
        assert!(html.contains("&quot;track_id&quot;: &quot;1440828878&quot;"));
        assert!(html.ends_with("</blockquote>"));
    }

    #[test]
    fn caption_defaults() {
        let meta = DumpCaptionMetadata {
            track_key: TrackKey::apple("i"),
            title: "",
            artist: "",
            album: "",
            duration: 61,
            bit_depth: 16,
            sample_rate: 44100,
            codec: None,
            genre: None,
            release_date: None,
            track_number: None,
            track_count: None,
        };
        let html = format_dump_caption(&meta);
        assert!(html.contains("<b></b> — "));
        assert!(html.contains("<i></i> · <code>1/1</code>"));
        assert!(html.contains("<code>ALAC · 16-bit · 44.1 kHz · 1:01</code>"));
    }

    #[test]
    fn roundtrip_through_parse() {
        let html = format_dump_caption(&sample_meta());
        let unescaped = html
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("<br/>", "\n");
        let parsed = parse_dump_caption(Some(&unescaped)).expect("payload found");
        assert_eq!(parsed.track_key, TrackKey::apple("1440828878"));
        assert_eq!(parsed.title, "Night Song");
        assert_eq!(parsed.artist, "A&R <duo>");
        assert_eq!(parsed.album, "Escapes");
        assert_eq!(parsed.duration, 215);
        assert_eq!(parsed.bit_depth, 24);
        assert_eq!(parsed.sample_rate, 48000);
        assert_eq!(parsed.genre, "Electronic");
        assert_eq!(parsed.release_date, "2021-06-04");
        assert_eq!(parsed.track_number, 2);
        assert_eq!(parsed.track_count, 10);
    }

    #[test]
    fn parse_rejects_missing_or_invalid() {
        assert!(parse_dump_caption(None).is_none());
        assert!(parse_dump_caption(Some("")).is_none());
        assert!(parse_dump_caption(Some("no payload here")).is_none());
        assert!(parse_dump_caption(Some("{\"title\": \"x\"}")).is_none());
        assert!(parse_dump_caption(Some("{\"provider\": \"apple\", \"track_id\": 123}")).is_none());
    }

    #[test]
    fn parse_defaults_missing_fields() {
        let parsed = parse_dump_caption(Some("{\"provider\": \"apple\", \"track_id\": \"abc\"}"))
            .expect("minimal payload parses");
        assert_eq!(parsed.track_key, TrackKey::apple("abc"));
        assert_eq!(parsed.title, "");
        assert_eq!(parsed.bit_depth, 16);
        assert_eq!(parsed.sample_rate, 44100);
        assert_eq!(parsed.genre, "Music");
        assert_eq!(parsed.track_number, 1);
        assert_eq!(parsed.track_count, 1);
    }

    #[test]
    fn payloads_without_required_fields_are_rejected() {
        assert!(
            parse_dump_caption(Some("{\"provider\": \"apple\", \"title\": \"Missing id\"}"))
                .is_none()
        );
    }
}
