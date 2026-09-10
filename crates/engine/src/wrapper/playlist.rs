//! HLS M3U8 playlist parsing for Apple Music ALAC streams.

use super::client::WrapperError;

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

/// Parse master HLS playlist to find the ALAC lossless audio stream variant.
pub fn parse_master_playlist(
    content: &str,
    master_url: &str,
) -> Result<AlacStreamInfo, WrapperError> {
    let mut alac_variant_uri = None;
    let mut sample_rate = 44100u32;
    let mut bit_depth = 16u32;

    // First, check #EXT-X-MEDIA for ALAC attributes (e.g. SAMPLE-RATE=44100, BIT-DEPTH=16)
    for line in content.lines() {
        let line = line.trim();
        if let Some(attrs) = line.strip_prefix("#EXT-X-MEDIA:") {
            let mut is_audio = false;
            let mut is_alac = false;
            let mut sr = None;
            let mut bd = None;

            for part in split_attributes(attrs) {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if k == "TYPE" && v == "AUDIO" {
                        is_audio = true;
                    }
                    if (k == "GROUP-ID" || k == "NAME") && v.to_ascii_lowercase().contains("alac") {
                        is_alac = true;
                    }
                    if k == "SAMPLE-RATE" {
                        sr = v.parse::<u32>().ok();
                    }
                    if k == "BIT-DEPTH" {
                        bd = v.parse::<u32>().ok();
                    }
                }
            }

            if is_audio && is_alac {
                if let Some(val) = sr {
                    sample_rate = val;
                }
                if let Some(val) = bd {
                    bit_depth = val;
                }
            }
        }
    }

    // Next, find the stream info with CODECS="alac"
    let mut next_is_alac = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#EXT-X-STREAM-INF:") {
            if line.contains("CODECS=\"alac\"") || line.contains("alac") {
                next_is_alac = true;
            }
            continue;
        }
        if next_is_alac && !line.starts_with('#') {
            alac_variant_uri = Some(resolve_url(master_url, line));
            break;
        }
    }

    let stream_url = alac_variant_uri.ok_or_else(|| {
        WrapperError::Message("No ALAC stream variant found in master playlist".into())
    })?;

    Ok(AlacStreamInfo {
        stream_url,
        codec: "alac".to_owned(),
        sample_rate,
        bit_depth,
    })
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

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") {
            let mut is_sample_aes = false;
            let mut key_uri = None;
            for part in split_attributes(attrs) {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if k == "METHOD" && v == "SAMPLE-AES" {
                        is_sample_aes = true;
                    } else if k == "URI" {
                        key_uri = Some(v.to_owned());
                    }
                }
            }
            if is_sample_aes {
                current_key_uri = key_uri;
            }
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
        let info = parse_master_playlist(master, base).expect("master playlist parse");
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
}
