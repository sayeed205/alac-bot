//! M4A tagging via ffmpeg. Port of `src/modules/alac/tagger.ts`.

use std::{
    future::Future,
    path::{Path, PathBuf},
};

use tracing::debug;

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
    // JS falsy: trackNumber 0 (or None) → 1.
    let n = meta.track_number.filter(|n| *n != 0).unwrap_or(1);
    let explicit = if meta.explicit { " [E]" } else { "" };
    let combined = format!("{n:02}. {} - {}{explicit} [ALAC]", meta.title, meta.artist);
    format!("{}.m4a", sanitize_filename(&combined))
}

/// A runner executes the assembled ffmpeg command. The production runner
/// shells out; tests record args and script exit codes.
pub trait FfmpegRunner: Send + Sync {
    /// Returns `Ok(())` on exit code 0; otherwise the exit code and the
    /// FULL stderr (the tagger slices the message itself).
    fn run(&self, args: &[String]) -> impl Future<Output = Result<(), (i32, String)>> + Send;
}

/// Production ffmpeg runner.
#[derive(Clone, Default)]
pub struct ProcessRunner;

impl FfmpegRunner for ProcessRunner {
    async fn run(&self, args: &[String]) -> Result<(), (i32, String)> {
        let output = tokio::process::Command::new(&args[0])
            .args(&args[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|error| (-1, error.to_string()))?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() {
            Ok(())
        } else {
            Err((output.status.code().unwrap_or(-1), stderr))
        }
    }
}

/// Tag a raw ALAC stream into an M4A with metadata, optional cover, and
/// optional lyrics. Returns the output path on success.
pub async fn tag_m4a_file<R: FfmpegRunner>(
    runner: &R,
    raw_audio_path: &Path,
    output_path: &Path,
    meta: &TrackMeta,
    cover: Option<&[u8]>,
    lyrics: Option<&str>,
) -> Result<PathBuf, TagError> {
    let cover_bytes = cover.filter(|c| !c.is_empty());
    // TS writes `{outputPath}.cover.jpg` (suffix appended to the full path).
    let temp_cover_path: Option<PathBuf> = cover_bytes.is_some().then(|| {
        let mut path = output_path.as_os_str().to_os_string();
        path.push(".cover.jpg");
        PathBuf::from(path)
    });
    if let Some(path) = &temp_cover_path {
        if let Some(cover) = cover_bytes {
            tokio::fs::write(path, cover)
                .await
                .map_err(|error| TagError::Message(error.to_string()))?;
        }
    }

    let mut args: Vec<String> = vec![
        "ffmpeg".to_owned(),
        "-y".to_owned(),
        "-i".to_owned(),
        raw_audio_path.to_string_lossy().into_owned(),
    ];

    if let Some(cover_path) = &temp_cover_path {
        args.push("-i".to_owned());
        args.push(cover_path.to_string_lossy().into_owned());
        args.extend([
            "-map".to_owned(),
            "0:a".to_owned(),
            "-map".to_owned(),
            "1".to_owned(),
            "-c".to_owned(),
            "copy".to_owned(),
            "-disposition:v:0".to_owned(),
            "attached_pic".to_owned(),
        ]);
    } else {
        args.extend(["-c".to_owned(), "copy".to_owned()]);
    }

    // Metadata conditionals use JS falsy semantics (empty/0 skip).
    if !meta.title.is_empty() {
        args.push("-metadata".into());
        args.push(format!("title={}", meta.title));
    }
    if !meta.artist.is_empty() {
        args.push("-metadata".into());
        args.push(format!("artist={}", meta.artist));
    }
    if !meta.album.is_empty() {
        args.push("-metadata".into());
        args.push(format!("album={}", meta.album));
    }
    if !meta.album_artist.is_empty() {
        args.push("-metadata".into());
        args.push(format!("album_artist={}", meta.album_artist));
    }
    if !meta.release_date.is_empty() {
        args.push("-metadata".into());
        args.push(format!("date={}", meta.release_date));
    }
    if let Some(genre) = &meta.genre {
        if !genre.is_empty() {
            args.push("-metadata".into());
            args.push(format!("genre={genre}"));
        }
    }
    if let Some(composer) = &meta.composer {
        if !composer.is_empty() {
            args.push("-metadata".into());
            args.push(format!("composer={composer}"));
        }
    }
    if let Some(track_number) = meta.track_number.filter(|n| *n != 0) {
        let track = match meta.track_count.filter(|c| *c != 0) {
            Some(count) => format!("{track_number}/{count}"),
            None => track_number.to_string(),
        };
        args.push("-metadata".into());
        args.push(format!("track={track}"));
    }
    if let Some(disc_number) = meta.disc_number.filter(|n| *n != 0) {
        let disc = match meta.disc_count.filter(|c| *c != 0) {
            Some(count) => format!("{disc_number}/{count}"),
            None => disc_number.to_string(),
        };
        args.push("-metadata".into());
        args.push(format!("disc={disc}"));
    }
    if let Some(lyrics) = lyrics.filter(|l| !l.is_empty()) {
        args.push("-metadata".into());
        args.push(format!("lyrics={lyrics}"));
    }

    args.push(output_path.to_string_lossy().into_owned());

    let result = runner.run(&args).await;

    // TS finally: cover temp removed in all paths.
    if let Some(cover_path) = &temp_cover_path {
        let _ = tokio::fs::remove_file(cover_path).await;
    }

    match result {
        Ok(()) => {
            debug!(output = %output_path.display(), "M4A tagged");
            Ok(output_path.to_owned())
        }
        Err((exit_code, stderr)) => {
            let tail: String = stderr
                .chars()
                .rev()
                .take(200)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Err(TagError::Message(format!(
                "FFmpeg tagging failed (exit code {exit_code}): {tail}"
            )))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("{0}")]
    Message(String),
}
