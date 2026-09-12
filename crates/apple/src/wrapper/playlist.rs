//! HLS M3U8 playlist parsing for Apple Music ALAC streams.

use music::CodecPreference;

use super::client::WrapperError;

/// One audio variant offered by a master playlist.
#[derive(Debug, Clone)]
pub struct StreamVariant {
    pub uri: String,
    pub codec: String,
    /// Audio group id, e.g. `audio-alac-stereo-96000-24`,
    /// `audio-stereo-256`, `audio-atmos-2768`.
    pub group_id: String,
    pub bandwidth: u64,
}

#[derive(Debug, Clone)]
pub struct AlacStreamInfo {
    pub stream_url: String,
    pub codec: String,
    pub sample_rate: u32,
    pub bit_depth: u32,
}

#[derive(Debug, Clone)]
pub struct MediaSegmentRef {
    pub uri: String,
    pub byte_range: Option<(u64, u64)>, // (offset, length)
    pub key_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaPlaylistInfo {
    pub init_uri: String,
    pub init_byte_range: Option<(u64, u64)>, // (offset, length)
    pub segments: Vec<MediaSegmentRef>,
    pub single_file_url: Option<String>,
    /// `METHOD` of the active `#EXT-X-KEY` (`SAMPLE-AES` or
    /// `ISO-23001-7`); `None` when the playlist carries no key.
    pub key_method: Option<String>,
    /// For CENC playlists: the base64 KID from the `data:;base64,...`
    /// key URI (the part after the comma).
    pub cenc_kid_b64: Option<String>,
}

/// Resolve a possibly relative URI against a base URL.
pub fn resolve_url(base: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_owned();
    }
    let base = base.split('?').next().unwrap_or(base);
    if let Some(pos) = base.rfind('/') {
        format!("{}/{}", &base[..pos], uri)
    } else {
        uri.to_owned()
    }
}

/// Parse byte range string like "1037@0" -> (offset: 0, length: 1037).
fn parse_byte_range(val: &str) -> Option<(u64, u64)> {
    let val = val.trim_matches('"').trim();
    if let Some((len_str, off_str)) = val.split_once('@') {
        let len: u64 = len_str.parse().ok()?;
        let off: u64 = off_str.parse().ok()?;
        Some((off, len))
    } else if let Ok(len) = val.parse::<u64>() {
        Some((0, len))
    } else {
        None
    }
}

/// Split comma-separated HLS tag attributes, ignoring commas within quotes.
fn split_attributes(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        let tail = s[start..].trim();
        if !tail.is_empty() {
            parts.push(tail);
        }
    }
    parts
}

/// Parse master HLS playlist and select the variant per `preference`.
///
/// ALAC variants carry `SAMPLE-RATE`/`BIT-DEPTH` on their `#EXT-X-MEDIA`
/// line; lossy variants encode their bitrate in the group id
/// (`audio-stereo-256`, `audio-atmos-2768`, `audio-HE-stereo-64`).
pub fn parse_master_playlist(
    content: &str,
    master_url: &str,
    preference: CodecPreference,
) -> Result<AlacStreamInfo, WrapperError> {
    let variants = collect_variants(content, master_url);
    select_variant(&variants, preference, content)
}

/// Extract every `#EXT-X-STREAM-INF` variant with its audio group.
fn collect_variants(content: &str, master_url: &str) -> Vec<StreamVariant> {
    let mut variants = Vec::new();
    let mut pending: Option<StreamVariant> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let mut variant = StreamVariant {
                uri: String::new(),
                codec: String::new(),
                group_id: String::new(),
                bandwidth: 0,
            };
            for part in split_attributes(attrs) {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if k == "CODECS" {
                        variant.codec = v.to_owned();
                    } else if k == "AUDIO" {
                        variant.group_id = v.to_owned();
                    } else if k == "BANDWIDTH" {
                        variant.bandwidth = v.parse().unwrap_or(0);
                    } else if k == "AVERAGE-BANDWIDTH" && variant.bandwidth == 0 {
                        // `BANDWIDTH` wins when present; the average is a
                        // stand-in only for variants missing the peak.
                        variant.bandwidth = v.parse().unwrap_or(0);
                    }
                }
            }
            pending = Some(variant);
        } else if pending.is_some() && !line.is_empty() && !line.starts_with('#') {
            let mut variant = pending.take().unwrap();
            variant.uri = resolve_url(master_url, line);
            variants.push(variant);
        }
    }
    variants
}

/// Quality rank of a group id. Higher is better; `None` = unusable.
/// Within a codec class the rank encodes bitrate so the group alone
/// disambiguates (`audio-atmos-2768` beats `audio-atmos-2448`,
/// `audio-stereo-256` beats `audio-stereo-128`, 24/192 ALAC beats 24/96).
fn group_rank(group_id: &str, codec: &str, preference: CodecPreference) -> Option<i64> {
    let is_atmos_group = group_id.starts_with("audio-atmos");
    match (codec, preference) {
        // Atmos mode wants the atmos group; lossless mode wants ALAC and
        // must not silently take an ec-3 stream.
        ("ec-3", CodecPreference::Atmos) if is_atmos_group => {
            group_bitrate(group_id, "audio-atmos-").map(|rate| 20_000 + rate)
        }
        ("alac", CodecPreference::HighestQuality) => {
            Some(10_000_000 + alac_group_specs(group_id).0 as i64)
        }
        ("mp4a.40.2", CodecPreference::HighestQuality) => {
            group_bitrate(group_id, "audio-stereo-").map(|rate| 5_000 + rate)
        }
        ("mp4a.40.5", CodecPreference::HighestQuality) => {
            group_bitrate(group_id, "audio-HE-stereo-").map(|rate| 4_000 + rate)
        }
        _ => None,
    }
}

/// Trailing integer of `prefix + number` in a group id, e.g.
/// `audio-atmos-2768` -> 2768.
fn group_bitrate(group_id: &str, prefix: &str) -> Option<i64> {
    group_id
        .strip_prefix(prefix)
        .and_then(|rest| rest.parse().ok())
}

/// Decode `audio-alac-stereo-96000-24` -> (96000, 24).
fn alac_group_specs(group_id: &str) -> (u32, u32) {
    let parts: Vec<&str> = group_id.split('-').collect();
    if parts.len() >= 2 {
        if let Ok(rate) = parts[parts.len() - 2].parse() {
            if let Ok(depth) = parts[parts.len() - 1].parse() {
                return (rate, depth);
            }
        }
    }
    (44_100, 16)
}

/// Pick the best variant per `preference`.
fn select_variant(
    variants: &[StreamVariant],
    preference: CodecPreference,
    content: &str,
) -> Result<AlacStreamInfo, WrapperError> {
    let mut best: Option<(i64, &StreamVariant, u32, u32)> = None;
    for variant in variants {
        let Some(rank) = group_rank(&variant.group_id, &variant.codec, preference) else {
            continue;
        };
        let (sample_rate, bit_depth) = if variant.codec == "alac" {
            alac_group_specs(&variant.group_id)
        } else {
            media_line_specs(content, &variant.group_id)
        };
        let better = match best {
            None => true,
            Some((best_rank, _, _, _)) => rank > best_rank,
        };
        if better {
            best = Some((rank, variant, sample_rate, bit_depth));
        }
    }

    let (_, variant, sample_rate, bit_depth) = best.ok_or_else(|| {
        let what = match preference {
            CodecPreference::Atmos => "Dolby Atmos",
            CodecPreference::HighestQuality => "no audio",
        };
        WrapperError::Message(format!("No {what} stream variant found in master playlist"))
    })?;

    Ok(AlacStreamInfo {
        stream_url: variant.uri.clone(),
        codec: variant.codec.clone(),
        sample_rate,
        bit_depth,
    })
}

/// `SAMPLE-RATE`/`BIT-DEPTH` from the `#EXT-X-MEDIA` line of `group_id`.
/// AAC defaults to 44.1/16; Atmos to 48/16 (16-channel JOC).
fn media_line_specs(content: &str, group_id: &str) -> (u32, u32) {
    let is_atmos = group_id.starts_with("audio-atmos");
    let mut sample_rate = None;
    let mut bit_depth = None;
    for line in content.lines() {
        let Some(attrs) = line.strip_prefix("#EXT-X-MEDIA:") else {
            continue;
        };
        let mut matches_group = false;
        let mut rate = None;
        let mut depth = None;
        for part in split_attributes(attrs) {
            if let Some((k, v)) = part.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                if k == "GROUP-ID" && v == group_id {
                    matches_group = true;
                } else if k == "SAMPLE-RATE" {
                    rate = v.parse::<u32>().ok();
                } else if k == "BIT-DEPTH" {
                    depth = v.parse::<u32>().ok();
                }
            }
        }
        if matches_group {
            sample_rate = rate.or(sample_rate);
            bit_depth = depth.or(bit_depth);
        }
    }
    if is_atmos {
        (sample_rate.unwrap_or(48_000), bit_depth.unwrap_or(16))
    } else {
        (sample_rate.unwrap_or(44_100), bit_depth.unwrap_or(16))
    }
}

/// Parse media HLS playlist to extract init segment and media segments.
pub fn parse_media_playlist(
    content: &str,
    media_url: &str,
) -> Result<MediaPlaylistInfo, WrapperError> {
    let mut init_uri = None;
    let mut init_byte_range = None;
    let mut segments = Vec::new();
    let mut current_key_uri = None;
    let mut next_range = None;
    let mut key_method = None;
    let mut cenc_kid_b64 = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") {
            let mut key_uri = None;
            let mut method = None;
            for part in split_attributes(attrs) {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if k == "METHOD" {
                        method = Some(v.to_owned());
                    } else if k == "URI" {
                        key_uri = Some(v.to_owned());
                    }
                }
            }
            match method.as_deref() {
                // FairPlay: URI is a skd:// template ref.
                Some("SAMPLE-AES") => current_key_uri = key_uri,
                // CENC: URI is "data:;base64,<kid>"; keep the KID.
                Some("ISO-23001-7") => {
                    current_key_uri = key_uri.clone();
                    if let Some(uri) = &key_uri {
                        cenc_kid_b64 = uri.rsplit(',').next().map(str::to_owned);
                    }
                }
                _ => {}
            }
            key_method = method;
            continue;
        }

        if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
            for part in split_attributes(attrs) {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if k == "URI" {
                        init_uri = Some(resolve_url(media_url, v));
                    } else if k == "BYTERANGE" {
                        init_byte_range = parse_byte_range(v);
                    }
                }
            }
            continue;
        }

        if let Some(range_str) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            next_range = parse_byte_range(range_str);
            continue;
        }

        if !line.starts_with('#') {
            let seg_url = resolve_url(media_url, line);
            segments.push(MediaSegmentRef {
                uri: seg_url,
                byte_range: next_range.take(),
                key_uri: current_key_uri.clone(),
            });
        }
    }

    let init_uri = init_uri.ok_or_else(|| {
        WrapperError::Message("No #EXT-X-MAP init segment found in media playlist".into())
    })?;

    // In Apple Music, typically all segments and the map point to the same single file
    let single_file = if segments.iter().all(|s| s.uri == init_uri) {
        Some(init_uri.clone())
    } else {
        None
    };

    Ok(MediaPlaylistInfo {
        init_uri,
        init_byte_range,
        segments,
        single_file_url: single_file,
        key_method,
        cenc_kid_b64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_master_and_media_playlist() {
        let master = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-alac-stereo-44100-16",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced",SAMPLE-RATE=44100,BIT-DEPTH=16
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=828199,CODECS="alac",AUDIO="audio-alac-stereo-44100-16"
P290437790_A1529441498_audio_en_gr1411.m3u8"#;

        let base = "https://aod.itunes.apple.com/assets/master.m3u8";
        let info = parse_master_playlist(master, base, CodecPreference::HighestQuality)
            .expect("master playlist parse");
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.bit_depth, 16);
        assert_eq!(
            info.stream_url,
            "https://aod.itunes.apple.com/assets/P290437790_A1529441498_audio_en_gr1411.m3u8"
        );

        let media = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://itunes.apple.com/P000000000/s1/e1",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1"
#EXT-X-MAP:URI="audio.mp4",BYTERANGE="1037@0"
#EXTINF:14.95365,
#EXT-X-BYTERANGE:1300622@1037
audio.mp4
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://itunes.apple.com/P290437790/c6",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1"
#EXTINF:14.95365,
#EXT-X-BYTERANGE:1532212@1301659
audio.mp4"#;

        let media_info =
            parse_media_playlist(media, &info.stream_url).expect("media playlist parse");
        assert_eq!(
            media_info.init_uri,
            "https://aod.itunes.apple.com/assets/audio.mp4"
        );
        assert_eq!(media_info.init_byte_range, Some((0, 1037)));
        assert_eq!(media_info.segments.len(), 2);
        assert_eq!(media_info.segments[0].byte_range, Some((1037, 1300622)));
        assert_eq!(
            media_info.segments[0].key_uri.as_deref(),
            Some("skd://itunes.apple.com/P000000000/s1/e1")
        );
        assert_eq!(media_info.segments[1].byte_range, Some((1301659, 1532212)));
        assert_eq!(
            media_info.segments[1].key_uri.as_deref(),
            Some("skd://itunes.apple.com/P290437790/c6")
        );
        assert_eq!(
            media_info.single_file_url,
            Some("https://aod.itunes.apple.com/assets/audio.mp4".to_string())
        );
    }

    /// A full store master with ALAC 24/96 + 24/48, AAC-256/128,
    /// HE-AAC-64 and Atmos variants (live Apple shapes).
    fn full_master() -> String {
        r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-stereo-256",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-alac-stereo-96000-24",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced",SAMPLE-RATE=96000,BIT-DEPTH=24
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-alac-stereo-48000-24",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced",SAMPLE-RATE=48000,BIT-DEPTH=24
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-alac-stereo-192000-24",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced",SAMPLE-RATE=192000,BIT-DEPTH=24
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-atmos-2768",AUTOSELECT=YES,CHANNELS="16/JOC",NAME="songEnhanced"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-atmos-2448",AUTOSELECT=YES,CHANNELS="16/JOC",NAME="songEnhanced"
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=263605,BANDWIDTH=270020,CODECS="mp4a.40.2",AUDIO="audio-stereo-256"
A_track_gr256.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=2858574,BANDWIDTH=3018699,CODECS="alac",AUDIO="audio-alac-stereo-96000-24"
A_track_alac_96.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=1751770,BANDWIDTH=1850877,CODECS="alac",AUDIO="audio-alac-stereo-48000-24"
A_track_alac_48.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=2858574,BANDWIDTH=3018699,CODECS="alac",AUDIO="audio-alac-stereo-192000-24"
A_track_alac_192.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=770158,BANDWIDTH=771138,CODECS="ec-3",AUDIO="audio-atmos-2768"
A_track_atmos_2768.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=450158,BANDWIDTH=450839,CODECS="ec-3",AUDIO="audio-atmos-2448"
A_track_atmos_2448.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=133444,BANDWIDTH=137870,CODECS="mp4a.40.2",AUDIO="audio-stereo-128"
A_track_gr128.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=69882,BANDWIDTH=78238,CODECS="mp4a.40.5",AUDIO="audio-HE-stereo-64"
A_track_gr64.m3u8"#
            .to_owned()
    }

    #[test]
    fn highest_quality_prefers_hi_res_alac() {
        let info = parse_master_playlist(
            &full_master(),
            "https://aod.itunes.apple.com/assets/master.m3u8",
            CodecPreference::HighestQuality,
        )
        .expect("parse");
        assert_eq!(info.codec, "alac");
        assert_eq!(info.sample_rate, 192_000);
        assert_eq!(info.bit_depth, 24);
        assert!(info.stream_url.ends_with("A_track_alac_192.m3u8"));
    }

    #[test]
    fn atmos_preference_picks_highest_atmos() {
        let info = parse_master_playlist(
            &full_master(),
            "https://aod.itunes.apple.com/assets/master.m3u8",
            CodecPreference::Atmos,
        )
        .expect("parse");
        assert_eq!(info.codec, "ec-3");
        assert_eq!(info.sample_rate, 48_000);
        assert!(info.stream_url.ends_with("A_track_atmos_2768.m3u8"));
    }

    #[test]
    fn aac_falls_back_when_no_alac() {
        let master = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-stereo-256",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-stereo-128",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced"
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=263605,BANDWIDTH=270020,CODECS="mp4a.40.2",AUDIO="audio-stereo-256"
A_track_gr256.m3u8
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=133444,BANDWIDTH=137870,CODECS="mp4a.40.2",AUDIO="audio-stereo-128"
A_track_gr128.m3u8"#;
        let info = parse_master_playlist(
            master,
            "https://aod.itunes.apple.com/assets/master.m3u8",
            CodecPreference::HighestQuality,
        )
        .expect("parse");
        assert_eq!(info.codec, "mp4a.40.2");
        assert!(info.stream_url.ends_with("A_track_gr256.m3u8"));
    }

    #[test]
    fn atmos_preference_without_atmos_is_an_error() {
        let master = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio-stereo-256",AUTOSELECT=YES,CHANNELS="2",NAME="songEnhanced"
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=263605,BANDWIDTH=270020,CODECS="mp4a.40.2",AUDIO="audio-stereo-256"
A_track_gr256.m3u8"#;
        let error = parse_master_playlist(
            master,
            "https://aod.itunes.apple.com/assets/master.m3u8",
            CodecPreference::Atmos,
        )
        .expect_err("no atmos variant");
        assert_eq!(
            error.to_string(),
            "No Dolby Atmos stream variant found in master playlist"
        );
    }
}
