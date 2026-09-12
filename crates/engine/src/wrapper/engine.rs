//! High-level native wrapper-lite engine.
//!
//! Coordinates:
//! 1. Fetching HLS playlist from wrapper-lite
//! 2. Parsing ALAC stream variant and segment byte ranges
//! 3. Fetching FairPlay key templates from wrapper-lite
//! 4. Downloading fragmented MP4 from Apple CDN
//! 5. Decrypting audio samples using Temari
//! 6. Returning an `AudioStreamSource` compatible with the ripper pipeline.

use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::Bytes;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::{
    client::WrapperLiteClient,
    decryptor::{decrypt_fragment, transform_init_segment},
    playlist::{parse_master_playlist, parse_media_playlist, AlacStreamInfo, CodecPreference},
};
use crate::streaming::{AudioStreamSource, ProgressCallback, StreamError};

pub struct WrapperEngine {
    client: WrapperLiteClient,
    http_client: reqwest::Client,
}

impl WrapperEngine {
    pub fn new(wrapper_url: &str, api_key: Option<&str>) -> Self {
        let client = WrapperLiteClient::new(wrapper_url, api_key);
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            http_client,
        }
    }

    pub fn client(&self) -> &WrapperLiteClient {
        &self.client
    }

    /// Primary entry point: rips/decrypts track from wrapper-lite, returning an `AudioStreamSource`.
    ///
    /// `preference` selects the variant when the master playlist offers
    /// several (ALAC vs AAC vs Atmos).
    pub async fn rip_track(
        &self,
        track_id: &str,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
        preference: CodecPreference,
    ) -> Result<AudioStreamSource, StreamError> {
        if signal.as_ref().is_some_and(|s| s.is_cancelled()) {
            return Err(StreamError::Message("Download was cancelled".into()));
        }

        if let Some(cb) = &on_progress {
            cb("Connecting to wrapper-lite engine...");
        }

        let master_url = self
            .client
            .fetch_m3u8_url(track_id)
            .await
            .map_err(|e| StreamError::Message(format!("Fetch m3u8 URL: {e}")))?;

        debug!(track_id = %track_id, master_url = %master_url, "Fetched master m3u8 URL");

        let master_resp = self
            .http_client
            .get(&master_url)
            .send()
            .await
            .map_err(|e| StreamError::Message(format!("Fetch master playlist: {e}")))?;
        let master_text = master_resp
            .text()
            .await
            .map_err(|e| StreamError::Message(format!("Read master playlist: {e}")))?;

        // 3. Parse and select the stream variant per preference. A store
        // with no lossless HLS hands out a direct file URL instead of a
        // master playlist; fall back to the web playback playlist.
        let stream_info = match parse_master_playlist(&master_text, &master_url, preference) {
            Ok(info) => info,
            Err(error) => {
                if !looks_like_master_playlist(&master_text) {
                    self.webplayback_fallback(track_id, signal.as_ref(), on_progress.as_ref())
                        .await?
                } else {
                    return Err(StreamError::Message(format!(
                        "Parse master playlist: {error}"
                    )));
                }
            }
        };
        let alac_info = stream_info;

        debug!(
            codec = %alac_info.codec,
            sample_rate = alac_info.sample_rate,
            bit_depth = alac_info.bit_depth,
            media_url = %alac_info.stream_url,
            "Selected ALAC stream variant"
        );

        if let Some(cb) = &on_progress {
            cb("Fetching media playlist and FairPlay keys...");
        }

        let media_resp = self
            .http_client
            .get(&alac_info.stream_url)
            .send()
            .await
            .map_err(|e| StreamError::Message(format!("Fetch media playlist: {e}")))?;
        let media_text = media_resp
            .text()
            .await
            .map_err(|e| StreamError::Message(format!("Read media playlist: {e}")))?;

        let media_info = parse_media_playlist(&media_text, &alac_info.stream_url)
            .map_err(|e| StreamError::Message(format!("Parse media playlist: {e}")))?;

        // 5b. CENC (Widevine) playlists — the webplayback AAC path — take a
        // different decryption route than the FairPlay master variants.
        if media_info.key_method.as_deref() == Some("ISO-23001-7") {
            let kid_b64 = media_info
                .cenc_kid_b64
                .clone()
                .ok_or_else(|| StreamError::Message("CENC playlist has no key id".into()))?;
            return self
                .rip_cenc_stream(
                    track_id,
                    &media_info,
                    &kid_b64,
                    signal.as_ref(),
                    on_progress.as_ref(),
                )
                .await;
        }

        let mut key_templates: HashMap<String, Arc<temari::rounds::Template>> = HashMap::new();
        for seg in &media_info.segments {
            if let Some(key_uri) = &seg.key_uri {
                if !key_templates.contains_key(key_uri) {
                    let adam_for_key =
                        if key_uri.contains("P000000000") || key_uri.ends_with("/s1/e1") {
                            "0"
                        } else {
                            track_id
                        };
                    let tmpl = self
                        .client
                        .fetch_template(adam_for_key, key_uri)
                        .await
                        .map_err(|e| {
                            StreamError::Message(format!("Fetch template for {key_uri}: {e}"))
                        })?;
                    key_templates.insert(key_uri.clone(), Arc::new(tmpl));
                }
            }
        }

        if let Some(cb) = &on_progress {
            cb("Downloading encrypted audio stream from Apple CDN...");
        }

        // In Apple Music, media_info.single_file_url is almost always present
        let decrypted_bytes = if let Some(single_url) = &media_info.single_file_url {
            let resp = self
                .http_client
                .get(single_url)
                .send()
                .await
                .map_err(|e| StreamError::Message(format!("Download audio stream: {e}")))?;
            let raw_data = resp
                .bytes()
                .await
                .map_err(|e| StreamError::Message(format!("Read audio stream bytes: {e}")))?;

            if let Some(cb) = &on_progress {
                cb("Decrypting FairPlay audio samples with Temari...");
            }

            self.decrypt_single_file_stream(&raw_data, &media_info, &key_templates, track_id)?
        } else {
            // Segment-by-segment download fallback
            self.decrypt_multi_segment_stream(
                &media_info,
                &key_templates,
                track_id,
                on_progress.as_ref(),
            )
            .await?
        };

        let total_size = decrypted_bytes.len() as u64;
        let stream = Box::pin(futures_util::stream::once(async move {
            Ok(Bytes::from(decrypted_bytes))
        }));

        Ok(AudioStreamSource {
            stream,
            source_name: format!("wrapper ({})", self.client.base_url()),
            codec: alac_info.codec,
            bit_depth: alac_info.bit_depth,
            sample_rate: alac_info.sample_rate,
            content_length: Some(total_size),
        })
    }

    /// Handles stores with no lossless HLS: `/m3u8` hands out a direct
    /// encrypted file while `/webplayback` still returns an AAC HLS
    /// playlist.
    async fn webplayback_fallback(
        &self,
        track_id: &str,
        signal: Option<&CancellationToken>,
        on_progress: Option<&ProgressCallback>,
    ) -> Result<AlacStreamInfo, StreamError> {
        if signal.is_some_and(|t| t.is_cancelled()) {
            return Err(StreamError::Message("Download was cancelled".into()));
        }
        if let Some(cb) = on_progress {
            cb("No lossless stream; using web playback...");
        }
        let media_url = self
            .client
            .fetch_webplayback(track_id)
            .await
            .map_err(|e| StreamError::Message(format!("Fetch web playback: {e}")))?;
        debug!(track_id = %track_id, media_url = %media_url, "Using web playback fallback");
        Ok(AlacStreamInfo {
            stream_url: media_url,
            codec: "mp4a.40.2".to_owned(),
            sample_rate: 44_100,
            bit_depth: 16,
        })
    }

    /// The CENC (Widevine) route: fetch a license from wrapper-lite,
    /// unwrap the content key, download the whole media file, decrypt each
    /// fragment with AES-CTR and return the reassembled stream.
    async fn rip_cenc_stream(
        &self,
        track_id: &str,
        media_info: &super::playlist::MediaPlaylistInfo,
        kid_b64: &str,
        signal: Option<&CancellationToken>,
        on_progress: Option<&ProgressCallback>,
    ) -> Result<AudioStreamSource, StreamError> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};

        if signal.is_some_and(|t| t.is_cancelled()) {
            return Err(StreamError::Message("Download was cancelled".into()));
        }
        if let Some(cb) = on_progress {
            cb("Fetching Widevine license for encrypted AAC...");
        }

        let kid = B64
            .decode(kid_b64)
            .map_err(|e| StreamError::Message(format!("Decode CENC key id: {e}")))?;
        let mut cdm = super::widevine::Cdm::new(&kid)
            .map_err(|e| StreamError::Message(format!("Init Widevine CDM: {e}")))?;
        let challenge = cdm
            .license_request()
            .map_err(|e| StreamError::Message(format!("Build license request: {e}")))?;

        // wrapper-lite forwards the playlist's original EXT-X-KEY URI
        // ("data:;base64,<kid>") verbatim to Apple; the PSSH built above
        // travels only inside the challenge.
        let key_uri = media_info
            .segments
            .iter()
            .find_map(|seg| seg.key_uri.clone())
            .unwrap_or_else(|| format!("data:;base64,{kid_b64}"));

        let license_b64 = self
            .client
            .fetch_license(track_id, &challenge, &key_uri)
            .await
            .map_err(|e| StreamError::Message(format!("Fetch license: {e}")))?;

        let content_keys = cdm
            .content_keys(&license_b64)
            .map_err(|e| StreamError::Message(format!("Unwrap license keys: {e}")))?;
        let content_key = content_keys
            .iter()
            .find(|k| k.key_id == kid)
            .map(|k| k.value)
            .or_else(|| content_keys.first().map(|k| k.value))
            .ok_or_else(|| StreamError::Message("License contained no content key".into()))?;

        if signal.is_some_and(|t| t.is_cancelled()) {
            return Err(StreamError::Message("Download was cancelled".into()));
        }
        if let Some(cb) = on_progress {
            cb("Downloading encrypted AAC stream...");
        }

        // Whole-file layout: init (EXT-X-MAP byterange) + fragments
        // (EXT-X-BYTERANGE) all reference one file.
        let single_url = media_info
            .single_file_url
            .as_deref()
            .or(media_info.segments.first().map(|s| s.uri.as_str()))
            .ok_or_else(|| StreamError::Message("CENC playlist has no segments".into()))?;
        let resp = self
            .http_client
            .get(single_url)
            .send()
            .await
            .map_err(|e| StreamError::Message(format!("Download audio stream: {e}")))?;
        let raw_data = resp
            .bytes()
            .await
            .map_err(|e| StreamError::Message(format!("Read audio stream bytes: {e}")))?;

        if let Some(cb) = on_progress {
            cb("Decrypting CENC audio samples...");
        }

        let (init_offset, init_len) = media_info.init_byte_range.unwrap_or((0, 1037));
        if raw_data.len() < (init_offset + init_len) as usize {
            return Err(StreamError::Message(
                "Raw stream shorter than init segment".into(),
            ));
        }
        let init_raw = &raw_data[init_offset as usize..(init_offset + init_len) as usize];
        let transformed_init = super::decryptor::transform_init_segment(init_raw)
            .map_err(|e| StreamError::Message(format!("Transform init segment: {e}")))?;

        let mut output = Vec::with_capacity(raw_data.len());
        output.extend_from_slice(&transformed_init);

        for (i, seg) in media_info.segments.iter().enumerate() {
            if signal.is_some_and(|t| t.is_cancelled()) {
                return Err(StreamError::Message("Download was cancelled".into()));
            }
            let (off, len) = seg
                .byte_range
                .ok_or_else(|| StreamError::Message(format!("Segment {i} missing byte range")))?;
            let start = off as usize;
            let end = (off + len) as usize;
            if end > raw_data.len() {
                return Err(StreamError::Message(format!(
                    "Segment {i} range {start}..{end} out of bounds ({})",
                    raw_data.len()
                )));
            }
            let mut frag = raw_data[start..end].to_vec();
            super::cenc::decrypt_cenc_fragment(&mut frag, &content_key)
                .map_err(|e| StreamError::Message(format!("Decrypt segment {i}: {e}")))?;
            super::cenc::strip_encryption_boxes(&mut frag)
                .map_err(|e| StreamError::Message(format!("Strip segment {i} boxes: {e}")))?;
            output.extend_from_slice(&frag);
        }

        let progressive = defragment_m4a_container(&output, track_id)?;
        let total_size = progressive.len() as u64;
        let stream = Box::pin(futures_util::stream::once(async move {
            Ok(Bytes::from(progressive))
        }));

        Ok(AudioStreamSource {
            stream,
            source_name: format!("wrapper ({})", self.client.base_url()),
            codec: "mp4a.40.2".to_owned(),
            bit_depth: 16,
            sample_rate: 44_100,
            content_length: Some(total_size),
        })
    }

    fn decrypt_single_file_stream(
        &self,
        raw_data: &[u8],
        media_info: &crate::wrapper::playlist::MediaPlaylistInfo,
        key_templates: &HashMap<String, Arc<temari::rounds::Template>>,
        track_id: &str,
    ) -> Result<Vec<u8>, StreamError> {
        // Extract and transform init segment
        let (init_offset, init_len) = media_info.init_byte_range.unwrap_or((0, 1037));
        if raw_data.len() < (init_offset + init_len) as usize {
            return Err(StreamError::Message(
                "Raw stream shorter than init segment".into(),
            ));
        }
        let init_raw = &raw_data[init_offset as usize..(init_offset + init_len) as usize];
        let transformed_init = transform_init_segment(init_raw)
            .map_err(|e| StreamError::Message(format!("Transform init segment: {e}")))?;

        let mut output = Vec::with_capacity(raw_data.len());
        output.extend_from_slice(&transformed_init);

        // Track default template if available
        let default_template = key_templates
            .iter()
            .find(|(k, _)| !k.contains("P000000000"))
            .map(|(_, v)| v.clone())
            .or_else(|| key_templates.values().next().cloned())
            .ok_or_else(|| {
                StreamError::Message("No FairPlay decryption template available".into())
            })?;

        // Process each media fragment
        for (i, seg) in media_info.segments.iter().enumerate() {
            let (off, len) = seg
                .byte_range
                .ok_or_else(|| StreamError::Message(format!("Segment {i} missing byte range")))?;
            let start = off as usize;
            let end = (off + len) as usize;
            if end > raw_data.len() {
                return Err(StreamError::Message(format!(
                    "Segment {i} range {}..{} out of bounds ({})",
                    start,
                    end,
                    raw_data.len()
                )));
            }
            let frag_raw = &raw_data[start..end];

            let tmpl = seg
                .key_uri
                .as_ref()
                .and_then(|uri| key_templates.get(uri))
                .unwrap_or(&default_template);

            let decrypted_frag = decrypt_fragment(frag_raw, tmpl)
                .map_err(|e| StreamError::Message(format!("Decrypt fragment {i}: {e}")))?;

            output.extend_from_slice(&decrypted_frag);
        }

        // Re-mux / defragment to standard progressive M4A container
        let progressive = defragment_m4a_container(&output, track_id)?;
        Ok(progressive)
    }

    async fn decrypt_multi_segment_stream(
        &self,
        media_info: &crate::wrapper::playlist::MediaPlaylistInfo,
        key_templates: &HashMap<String, Arc<temari::rounds::Template>>,
        track_id: &str,
        _: Option<&ProgressCallback>,
    ) -> Result<Vec<u8>, StreamError> {
        let init_resp = self
            .http_client
            .get(&media_info.init_uri)
            .send()
            .await
            .map_err(|e| StreamError::Message(format!("Fetch init segment: {e}")))?;
        let init_bytes = init_resp
            .bytes()
            .await
            .map_err(|e| StreamError::Message(format!("Read init bytes: {e}")))?;
        let transformed_init = transform_init_segment(&init_bytes)
            .map_err(|e| StreamError::Message(format!("Transform init segment: {e}")))?;

        let mut output = Vec::new();
        output.extend_from_slice(&transformed_init);

        let default_template = key_templates
            .iter()
            .find(|(k, _)| !k.contains("P000000000"))
            .map(|(_, v)| v.clone())
            .or_else(|| key_templates.values().next().cloned())
            .ok_or_else(|| StreamError::Message("No FairPlay template available".into()))?;

        for (i, seg) in media_info.segments.iter().enumerate() {
            let seg_resp = self
                .http_client
                .get(&seg.uri)
                .send()
                .await
                .map_err(|e| StreamError::Message(format!("Fetch segment {i}: {e}")))?;
            let seg_bytes = seg_resp
                .bytes()
                .await
                .map_err(|e| StreamError::Message(format!("Read segment {i}: {e}")))?;

            let tmpl = seg
                .key_uri
                .as_ref()
                .and_then(|uri| key_templates.get(uri))
                .unwrap_or(&default_template);

            let dec = decrypt_fragment(&seg_bytes, tmpl)
                .map_err(|e| StreamError::Message(format!("Decrypt segment {i}: {e}")))?;
            output.extend_from_slice(&dec);
        }

        let progressive = defragment_m4a_container(&output, track_id)?;
        Ok(progressive)
    }
}

/// True when the response is a master playlist rather than a direct
/// media file. A master starts with `#EXTM3U` and carries
/// `#EXT-X-STREAM-INF` lines.
fn looks_like_master_playlist(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with("#EXTM3U") && text.contains("#EXT-X-STREAM-INF")
}

/// Helper to ensure the decrypted fMP4 is finalized into a standard progressive M4A container.
/// Uses MP4Box or ffmpeg if present for zero-loss, rapid container defragmentation;
/// falls back to the decrypted fMP4 directly if external remuxer is absent.
fn defragment_m4a_container(fmp4_data: &[u8], track_id: &str) -> Result<Vec<u8>, StreamError> {
    use std::process::Command;

    let temp_dir = std::env::temp_dir();
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let in_path = temp_dir.join(format!("raw_fmp4_{}_{}.m4a", track_id, unique_id));
    let out_path = temp_dir.join(format!("clean_m4a_{}_{}.m4a", track_id, unique_id));

    if let Err(e) = std::fs::write(&in_path, fmp4_data) {
        warn!(error = %e, "Failed to write temp fmp4, returning raw decrypted bytes");
        return Ok(fmp4_data.to_vec());
    }

    // Try MP4Box first (exact tool used by apple-music-downloader)
    let mp4box_status = Command::new("MP4Box")
        .args([
            "-inter",
            "500",
            in_path.to_str().unwrap_or(""),
            "-out",
            out_path.to_str().unwrap_or(""),
        ])
        .output();

    let success = match mp4box_status {
        Ok(output) if output.status.success() && out_path.exists() => true,
        _ => {
            // Fallback to ffmpeg stream copy
            let ffmpeg_status = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-i",
                    in_path.to_str().unwrap_or(""),
                    "-c",
                    "copy",
                    out_path.to_str().unwrap_or(""),
                ])
                .output();
            matches!(ffmpeg_status, Ok(output) if output.status.success() && out_path.exists())
        }
    };

    let result = if success {
        match std::fs::read(&out_path) {
            Ok(bytes) => bytes,
            Err(_) => fmp4_data.to_vec(),
        }
    } else {
        debug!("Remux command not available or failed, returning direct decrypted fmp4");
        fmp4_data.to_vec()
    };

    let _ = std::fs::remove_file(&in_path);
    let _ = std::fs::remove_file(&out_path);

    Ok(result)
}
