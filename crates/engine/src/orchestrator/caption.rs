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

/// Extracts the current structured metadata from a message caption. Returns
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

/// TS regex: `/(\{[\s\S]*?"id"\s*:\s*"[^"]+"[\s\S]*?\})/s`.
///
/// Regex semantics, reproduced exactly: scan `{` positions left to right;
/// for each start, try `"id"` occurrences in order (lazy middle span) —
/// each must be followed by `\s*:\s*"` + a non-empty `[^"]+` + `"`, and a
/// `}` must exist after the value (lazy tail takes the first one). The
/// first start with a completing occurrence wins; `\s*` may be empty.
fn extract_payload(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for (start, _) in text.match_indices('{') {
        let mut q = start + 1;
        while q <= text.len() {
            let Some(rel) = text[q..].find("\"track_id\"") else {
                break;
            };
            let id_at = q + rel;
            let mut i = id_at + 10; // past the literal `"track_id"`
            while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b':' {
                i += 1;
                while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'"' {
                    i += 1;
                    let value_start = i;
                    while i < bytes.len() && bytes[i] != b'"' {
                        i += 1;
                    }
                    // `[^"]+` needs a non-empty value and a closing quote.
                    if i > value_start && i < bytes.len() {
                        let after_value = i + 1;
                        // Lazy tail: the first `}` at or after the value.
                        if let Some(rel) = text[after_value..].find('}') {
                            let end = after_value + rel;
                            return Some(text[start..=end].to_string());
                        }
                    }
                }
            }
            // This occurrence cannot complete the pattern — backtrack to
            // the next `"id"` occurrence for this start.
            q = id_at + 1;
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
        // parse_dump_caption reads the payload from raw text; strip the HTML
        // escaping the way Telegram text would present it. Simpler: build a
        // plain-text version by un-escaping.
        let unescaped = html
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            // Telegram presents <br/> as a line break in message text.
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
        // track_id present but not a string.
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

    #[test]
    fn escape_covers_all_five_chars() {
        assert_eq!(html_escape("a&<>\"'z"), "a&amp;&lt;&gt;&quot;&#39;z");
    }
}
