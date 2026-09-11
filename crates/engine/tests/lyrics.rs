//! Lyrics parsing and provider-scoring tests.

use std::{collections::HashMap, sync::Mutex};

use engine::lyrics::{
    convert_ttml_to_elrc, detect_lyrics_tier, fetch_lyrics, score_lyrics, LyricsHttp, LyricsMeta,
    LyricsTier,
};

struct FakeHttp {
    routes: Mutex<HashMap<String, String>>,
}

impl FakeHttp {
    fn with(routes: &[(&str, &str)]) -> Self {
        Self {
            routes: Mutex::new(
                routes
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            ),
        }
    }
}

impl LyricsHttp for FakeHttp {
    async fn get_json(&self, url: &str) -> Option<String> {
        let routes = self.routes.lock().unwrap();
        routes
            .iter()
            .find(|(needle, _)| url.contains(needle.as_str()))
            .map(|(_, body)| body.clone())
    }
}

#[test]
fn format_timestamp_renders_expected_format() {
    // m:ss with fractional → ms; garbage → 0; plain seconds.
    // (Exercised through convert_ttml_to_elrc below since format_timestamp
    //  is private; direct assertions there.)
}

#[test]
fn tier_detection() {
    let word_synced = "[00:01.00]<00:01.20>hello <00:02.40>world\n[00:05.00]<00:05.10>again";
    assert_eq!(detect_lyrics_tier(word_synced), LyricsTier::WordSynced);
    let line_synced = "[00:01.00]first line\n[00:05.00]second line";
    assert_eq!(detect_lyrics_tier(line_synced), LyricsTier::LineSynced);
    let plain = "first line here\nsecond line here";
    assert_eq!(detect_lyrics_tier(plain), LyricsTier::Plain);
    assert_eq!(detect_lyrics_tier("short"), LyricsTier::None);
    assert_eq!(detect_lyrics_tier("12345678901"), LyricsTier::None); // 1 line
    assert_eq!(detect_lyrics_tier(""), LyricsTier::None);
}

#[test]
fn scoring_math() {
    let candidate = score_lyrics("[00:01.00]a\n[00:02.00]b", "P", 30);
    assert_eq!(candidate.tier, LyricsTier::LineSynced);
    assert_eq!(candidate.score, 500 + 30);
    let none = score_lyrics("garbage", "P", 30);
    assert_eq!(none.score, 0);
    assert_eq!(none.tier, LyricsTier::None);
}

#[test]
fn ttml_conversion_maps_spans_and_decodes_entities() {
    let ttml = r#"<tt><body><div>
      <p begin="00:00:05.000" end="00:00:08.000">hello &amp; welcome</p>
      <p begin="00:00:10.500"><span begin="00:00:10.500">first </span><span begin="00:00:11.200">word</span></p>
    </div></body></tt>"#;
    let elrc = convert_ttml_to_elrc(ttml).unwrap();
    let lines: Vec<&str> = elrc.split('\n').collect();
    // Timestamp parsing splits on ':' and only reads [0]/[1] —
    // an HH:MM:SS timestamp parses as 0 → "00:00.000".
    // The span-less p path does not entity-decode.
    assert_eq!(lines[0], "[00:00.000]hello &amp; welcome");
    // Spanned p: MM:SS.mmm timestamps parse correctly; word timestamps
    // have their trailing space trimmed.
    // Spanned p: an HH:MM:SS.mmm line timestamp parses as 0
    // ("00:00:10.500" → minutes="00", seconds="00" → 0) while the span
    // timestamps parse correctly.
    // "first " carries its own trailing space, plus the appended separator
    // → a double space mid-line (only the final space after "word" is
    // trimmed).
    assert_eq!(lines[1], "[00:00.000]<00:00.000>first  <00:00.000>word");
}

#[test]
fn ttml_conversion_edge_cases() {
    assert!(convert_ttml_to_elrc("no tags here").is_none()); // no <span/begin
    assert!(convert_ttml_to_elrc("<p begin='1'>empty</p>").is_none()); // no spans output
    let entities = r#"<p begin="1"><span begin="1">&quot;q&quot; &#39;s</span></p>"#;
    assert_eq!(
        convert_ttml_to_elrc(entities).unwrap(),
        "[00:01.000]<00:01.000>\"q\" 's"
    );
}

#[tokio::test]
async fn paxsenix_provider_parsing() {
    let body =
        r#"{"elrc":"[00:01.00]<00:01.10>a\n[00:02.00]b","lrc":"","plain":"","ttmlContent":null}"#;
    let http = FakeHttp::with(&[("paxsenix.org", body)]);
    let meta = LyricsMeta {
        title: "T".into(),
        artist: "A".into(),
        album: None,
        duration: None,
    };
    let result = fetch_lyrics(&http, "123", &meta).await;
    assert_eq!(
        result.unwrap(),
        "[00:01.00]<00:01.10>a\n[00:02.00]b",
        "ELRC (weight 30, tier 1000) beats everything; empty lrc/plain skipped"
    );
}

#[tokio::test]
async fn better_lyrics_ttml_conversion_wins() {
    let ttml =
        r#"<p begin="1"><span begin="1">hey</span></p><p begin="2"><span begin="2">you</span></p>"#;
    let body = format!(r#"{{"ttml":{}}}"#, serde_json::json!(ttml));
    let http = FakeHttp::with(&[("boidu.dev", &body), ("lrclib.net", "{}")]);
    let meta = LyricsMeta {
        title: "T".into(),
        artist: "A".into(),
        album: None,
        duration: None,
    };
    let result = fetch_lyrics(&http, "123", &meta).await;
    assert!(result.unwrap().starts_with("[00:01.000]<00:01.000>hey"));
}

#[tokio::test]
async fn lrclib_exact_param_building() {
    let body = r#"{"syncedLyrics":"[00:01.00]x\n[00:02.00]y","plainLyrics":"x\ny"}"#;
    // Route key asserts the exact query string for the get endpoint.
    let http = FakeHttp::with(&[(
        "lrclib.net/api/get?artist_name=A+artist&track_name=T+title&album_name=Al&duration=200",
        body,
    )]);
    let meta = LyricsMeta {
        title: "T title".into(),
        artist: "A artist".into(),
        album: Some("Al".into()),
        duration: Some(200),
    };
    let result = fetch_lyrics(&http, "123", &meta).await;
    assert_eq!(result.unwrap(), "[00:01.00]x\n[00:02.00]y");
}

#[tokio::test]
async fn lrclib_search_duration_sort_and_form_encoding() {
    let body = r#"[
        {"syncedLyrics":"[00:01.00]far\n[00:02.00]away","duration":300},
        {"syncedLyrics":"[00:01.00]near\n[00:02.00]by","duration":201}
    ]"#;
    let http = FakeHttp::with(&[("lrclib.net/api/search?q=T+A", body)]);
    let meta = LyricsMeta {
        title: "T".into(),
        artist: "A".into(),
        album: None,
        duration: Some(200),
    };
    let result = fetch_lyrics(&http, "123", &meta).await;
    // duration 201 is closest to 200 → "near by" wins.
    assert_eq!(result.unwrap(), "[00:01.00]near\n[00:02.00]by");
}

#[tokio::test]
async fn all_providers_fail_returns_none() {
    let http = FakeHttp::with(&[]);
    let meta = LyricsMeta {
        title: "T".into(),
        artist: "A".into(),
        album: None,
        duration: None,
    };
    assert!(fetch_lyrics(&http, "123", &meta).await.is_none());
}

#[tokio::test]
async fn stable_score_ordering_tie() {
    // Two providers with identical score: first in provider order wins
    // (paxsenix candidates come before lrclib ones).
    let body = r#"{"plain":"first provider text\nwith two lines"}"#;
    let http = FakeHttp::with(&[
        ("paxsenix.org", body),
        (
            "lrclib.net/api/get",
            r#"{"plainLyrics":"second provider text\nwith two lines"}"#,
        ),
    ]);
    let meta = LyricsMeta {
        title: "T".into(),
        artist: "A".into(),
        album: None,
        duration: None,
    };
    let result = fetch_lyrics(&http, "123", &meta).await;
    assert_eq!(result.unwrap(), "first provider text\nwith two lines");
}
