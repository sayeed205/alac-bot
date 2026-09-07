//! Lyrics acquisition: tier detection, TTML→ELRC conversion, scoring, and
//! four providers racing concurrently. Port of `src/modules/alac/lyrics.ts`.

use std::{future::Future, sync::OnceLock};

use regex::Regex;
use tracing::debug;

/// Quality tiers; numeric values feed the scoring math (TS parity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsTier {
    WordSynced = 1000,
    LineSynced = 500,
    Plain = 100,
    None = 0,
}

impl LyricsTier {
    fn is_none(self) -> bool {
        matches!(self, LyricsTier::None)
    }
}

#[derive(Debug, Clone)]
pub struct LyricsCandidate {
    pub text: String,
    pub provider: &'static str,
    pub tier: LyricsTier,
    pub score: i64,
}

/// Metadata the lyrics providers use to locate a track.
#[derive(Debug, Clone, Default)]
pub struct LyricsMeta {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<i64>,
}

fn word_sync_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<\d{1,3}:\d{2}(?:[.:]\d{2,3})?>").expect("word sync regex"))
}

fn line_sync_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\d{1,3}:\d{2}(?:[.:]\d{2,3})?]").expect("line sync regex"))
}

fn p_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<p\b[^>]*\bbegin=["']([^"']+)["'][^>]*>(.*?)</p>"#).expect("p tag regex")
    })
}

fn span_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<span\b[^>]*\bbegin=["']([^"']+)["'][^>]*>(.*?)</span>"#)
            .expect("span tag regex")
    })
}

fn any_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<[^>]+>").expect("any tag regex"))
}

/// Parse `m:ss` or seconds (possibly fractional) → total seconds; NaN → 0.
fn parse_time(time_str: &str) -> f64 {
    if time_str.contains(':') {
        let parts: Vec<&str> = time_str.split(':').collect();
        let min_part = parts
            .first()
            .and_then(|p| p.parse::<f64>().ok())
            .unwrap_or(0.0);
        let sec_part = parts
            .get(1)
            .and_then(|p| p.parse::<f64>().ok())
            .unwrap_or(0.0);
        min_part * 60.0 + sec_part
    } else {
        time_str.parse::<f64>().unwrap_or(0.0)
    }
}

/// Format seconds as `mm:ss.mmm` (TS `formatTimestamp` parity).
fn format_timestamp(time_str: &str) -> String {
    let total_sec = parse_time(time_str);
    let min = (total_sec / 60.0).floor();
    let sec = (total_sec % 60.0).floor();
    let ms = ((total_sec % 1.0) * 1000.0).round().floor();
    format!("{min:02}:{sec:02}.{ms:03}")
}

/// Convert Apple Music TTML with word-level spans into Enhanced LRC.
pub fn convert_ttml_to_elrc(ttml: &str) -> Option<String> {
    if !ttml.contains("<span") || !ttml.contains("begin=") {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    for p_caps in p_tag_regex().captures_iter(ttml) {
        let line_start = format_timestamp(&p_caps[1]);
        let inner = &p_caps[2];

        let mut elrc_line = format!("[{line_start}]");
        let mut word_count = 0usize;

        for span_caps in span_tag_regex().captures_iter(inner) {
            word_count += 1;
            let word_start = format_timestamp(&span_caps[1]);
            let word_text = decode_entities(&strip_tags(&span_caps[2]));
            elrc_line.push_str(&format!("<{word_start}>{word_text} "));
        }

        if word_count > 0 {
            lines.push(elrc_line.trim_end().to_owned());
        } else {
            let clean_line = strip_tags(inner).trim().to_owned();
            if !clean_line.is_empty() {
                lines.push(format!("[{line_start}]{clean_line}"));
            }
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn strip_tags(text: &str) -> String {
    any_tag_regex().replace_all(text, "").into_owned()
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Classify lyrics quality.
pub fn detect_lyrics_tier(lyrics: &str) -> LyricsTier {
    let trimmed = lyrics.trim();
    if trimmed.len() < 10 {
        return LyricsTier::None;
    }
    let lines: Vec<&str> = trimmed
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.len() < 2 {
        return LyricsTier::None;
    }
    if lines.iter().any(|l| word_sync_regex().is_match(l)) {
        return LyricsTier::WordSynced;
    }
    if lines.iter().any(|l| line_sync_regex().is_match(l)) {
        return LyricsTier::LineSynced;
    }
    LyricsTier::Plain
}

/// Score a lyrics text from one provider (TS `scoreLyrics` parity).
pub fn score_lyrics(text: &str, provider: &'static str, provider_weight: i64) -> LyricsCandidate {
    let tier = detect_lyrics_tier(text);
    let score = if tier.is_none() {
        0
    } else {
        tier as i64 + provider_weight
    };
    LyricsCandidate {
        text: text.trim().to_owned(),
        provider,
        tier,
        score,
    }
}

/// The HTTP seam all lyrics providers cross. `None` = any failure (TS
/// catches every fetch/parse error and returns no candidates).
pub trait LyricsHttp: Send + Sync {
    fn get_json(&self, url: &str) -> impl Future<Output = Option<String>> + Send;
}

/// URL-encode with `encodeURIComponent` semantics (JS keeps
/// `A-Za-z0-9-_.!~*'()` bare and escapes the rest).
fn encode_uri_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// URL-encode with `URLSearchParams` semantics (space → `+`; only
/// `A-Za-z0-9-_.~` kept bare). This differs from encodeURIComponent.
fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

async fn fetch_paxsenix(http: &impl LyricsHttp, track_id: &str) -> Vec<LyricsCandidate> {
    let url = format!(
        "https://lyrics.paxsenix.org/apple-music/lyrics?id={}",
        encode_uri_component(track_id)
    );
    let Some(body) = http.get_json(&url).await else {
        return Vec::new();
    };
    let data: Option<serde_json::Value> = serde_json::from_str(&body).ok();
    let Some(data) = data else {
        return Vec::new();
    };
    let str_field = |name: &str| {
        data.get(name)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .filter(|s| !s.trim().is_empty())
    };
    let mut candidates = Vec::new();
    if let Some(elrc) = str_field("elrc") {
        candidates.push(score_lyrics(&elrc, "Paxsenix Apple Music (ELRC)", 30));
    }
    if let Some(ttml) = str_field("ttmlContent") {
        if let Some(converted) = convert_ttml_to_elrc(&ttml) {
            candidates.push(score_lyrics(
                &converted,
                "Paxsenix Apple Music (TTML-ELRC)",
                30,
            ));
        }
    }
    if let Some(lrc) = str_field("lrc") {
        candidates.push(score_lyrics(&lrc, "Paxsenix Apple Music (LRC)", 25));
    }
    if let Some(plain) = str_field("plain") {
        candidates.push(score_lyrics(&plain, "Paxsenix Apple Music (Plain)", 20));
    }
    candidates
}

async fn fetch_better_lyrics(http: &impl LyricsHttp, meta: &LyricsMeta) -> Vec<LyricsCandidate> {
    let url = format!(
        "https://lyrics-api.boidu.dev/getLyrics?s={}&a={}",
        encode_uri_component(&meta.title),
        encode_uri_component(&meta.artist)
    );
    let Some(body) = http.get_json(&url).await else {
        return Vec::new();
    };
    let data: Option<serde_json::Value> = serde_json::from_str(&body).ok();
    let Some(data) = data else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if let Some(ttml) = data
        .get("ttml")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if let Some(converted) = convert_ttml_to_elrc(ttml) {
            candidates.push(score_lyrics(&converted, "BetterLyrics (Word Synced)", 25));
        }
    }
    if let Some(lrc) = data
        .get("lrc")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(score_lyrics(lrc, "BetterLyrics (LRC)", 20));
    }
    candidates
}

async fn fetch_lrclib_exact(http: &impl LyricsHttp, meta: &LyricsMeta) -> Vec<LyricsCandidate> {
    let mut params = format!(
        "artist_name={}&track_name={}",
        form_encode(&meta.artist),
        form_encode(&meta.title)
    );
    if let Some(album) = &meta.album {
        params.push_str(&format!("&album_name={}", form_encode(album)));
    }
    if let Some(duration) = meta.duration {
        if duration > 0 {
            params.push_str(&format!("&duration={duration}"));
        }
    }
    let url = format!("https://lrclib.net/api/get?{params}");
    let Some(body) = http.get_json(&url).await else {
        return Vec::new();
    };
    let data: Option<serde_json::Value> = serde_json::from_str(&body).ok();
    let Some(data) = data else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if let Some(synced) = data
        .get("syncedLyrics")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(score_lyrics(synced, "LRCLIB Exact (LRC)", 15));
    }
    if let Some(plain) = data
        .get("plainLyrics")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(score_lyrics(plain, "LRCLIB Exact (Plain)", 10));
    }
    candidates
}

async fn fetch_lrclib_search(http: &impl LyricsHttp, meta: &LyricsMeta) -> Vec<LyricsCandidate> {
    let url = format!(
        "https://lrclib.net/api/search?q={}",
        form_encode(&format!("{} {}", meta.title, meta.artist))
    );
    let Some(body) = http.get_json(&url).await else {
        return Vec::new();
    };
    let Ok(results) = serde_json::from_str::<Vec<serde_json::Value>>(&body) else {
        return Vec::new();
    };
    if results.is_empty() {
        return Vec::new();
    }

    // TS: sort by |duration - target| when a target exists (stable).
    let mut sorted = results;
    if let Some(target) = meta.duration {
        if target > 0 {
            sorted.sort_by_key(|item| {
                let duration = item.get("duration").and_then(|v| v.as_i64()).unwrap_or(0);
                (duration - target).abs()
            });
        }
    }
    let Some(best) = sorted.into_iter().next() else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    if let Some(synced) = best
        .get("syncedLyrics")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(score_lyrics(synced, "LRCLIB Search (LRC)", 5));
    }
    if let Some(plain) = best
        .get("plainLyrics")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(score_lyrics(plain, "LRCLIB Search (Plain)", 0));
    }
    candidates
}

/// Race all four providers concurrently and return the best candidate's
/// text, or `None` when nothing usable was found.
pub async fn fetch_lyrics(
    http: &impl LyricsHttp,
    track_id: &str,
    meta: &LyricsMeta,
) -> Option<String> {
    let (paxsenix, better, exact, search) = tokio::join!(
        fetch_paxsenix(http, track_id),
        fetch_better_lyrics(http, meta),
        fetch_lrclib_exact(http, meta),
        fetch_lrclib_search(http, meta),
    );

    let all_candidates: Vec<LyricsCandidate> = paxsenix
        .into_iter()
        .chain(better)
        .chain(exact)
        .chain(search)
        .collect();

    let mut valid: Vec<LyricsCandidate> = all_candidates
        .into_iter()
        .filter(|c| !c.tier.is_none() && c.score > 0)
        .collect();

    if valid.is_empty() {
        return None;
    }
    // Stable sort descending by score (TS Array.sort comparator parity —
    // sort_by_key is stable, matching TS's unspecified-stable behavior).
    valid.sort_by_key(|candidate| std::cmp::Reverse(candidate.score));
    let top = valid.into_iter().next()?;
    debug!(
        provider = top.provider,
        score = top.score,
        "Lyrics candidate selected"
    );
    Some(top.text)
}
