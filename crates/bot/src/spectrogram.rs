//! Audio probing and spectrogram generation used by the `/spec` command.

use std::{
    path::Path,
    process::{ExitStatus, Output, Stdio},
    sync::{Arc, OnceLock},
};

use engine::limits::{MAX_PROCESS_OUTPUT_BYTES, PROCESS_TIMEOUT};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Semaphore,
};

static SPECTROGRAM_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn spectrogram_slot() -> Arc<Semaphore> {
    SPECTROGRAM_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(2)))
        .clone()
}

async fn read_limited<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Ok(output);
        }
        let remaining = MAX_PROCESS_OUTPUT_BYTES.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..read.min(remaining)]);
    }
}

async fn run_bounded(mut command: Command) -> Result<Output, String> {
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("process stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("process stderr unavailable")?;
    let stdout_task = tokio::spawn(read_limited(stdout));
    let stderr_task = tokio::spawn(read_limited(stderr));
    let status: ExitStatus = match tokio::time::timeout(PROCESS_TIMEOUT, child.wait()).await {
        Ok(status) => status.map_err(|e| e.to_string())?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err("audio process timed out".to_owned());
        }
    };
    let stdout = stdout_task.await.map_err(|e| e.to_string())??;
    let stderr = stderr_task.await.map_err(|e| e.to_string())??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Metadata extracted from the first stream in an audio file.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioProbeResult {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub codec: String,
    pub sample_rate: u32,
    pub bit_depth: Option<u32>,
    pub channels: u32,
    pub bit_rate: Option<u64>,
    pub duration: f64,
}

/// Extensions accepted as audio documents by the command.
pub const AUDIO_EXTENSIONS: [&str; 18] = [
    ".m4a", ".flac", ".mp3", ".wav", ".wave", ".aac", ".alac", ".ogg", ".oga", ".opus", ".aiff",
    ".aif", ".wma", ".mka", ".ape", ".wv", ".dsf", ".dff",
];

fn number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn string(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    })
}

fn tag(
    stream: Option<&serde_json::Value>,
    format: Option<&serde_json::Value>,
    name: &str,
) -> Option<String> {
    for object in [stream, format].into_iter().flatten() {
        if let Some(tags) = object.get("tags").and_then(serde_json::Value::as_object) {
            if let Some(value) = tags.iter().find_map(|(key, value)| {
                key.eq_ignore_ascii_case(name)
                    .then(|| string(Some(value)))
                    .flatten()
            }) {
                return Some(value);
            }
        }
    }
    None
}

/// Probe audio metadata with ffprobe.
pub async fn probe_audio(file: &Path) -> Result<AudioProbeResult, String> {
    let _permit = spectrogram_slot()
        .acquire_owned()
        .await
        .map_err(|_| "spectrogram worker unavailable".to_owned())?;
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_bounded(command).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("ffprobe exited with {}", output.status)
        } else {
            stderr
        });
    }

    let data: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    let stream = data
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .and_then(|v| v.first());
    let format = data.get("format");

    let sample_rate = number(stream.and_then(|v| v.get("sample_rate")))
        .filter(|value| *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(44_100);
    let channels = number(stream.and_then(|v| v.get("channels")))
        .filter(|value| *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(2);
    let bit_depth = number(stream.and_then(|v| v.get("bits_per_raw_sample")))
        .or_else(|| number(stream.and_then(|v| v.get("bits_per_sample"))))
        .filter(|value| *value > 0.0)
        .map(|value| value as u32);
    let duration = number(stream.and_then(|v| v.get("duration")))
        .or_else(|| number(format.and_then(|v| v.get("duration"))))
        .filter(|value| *value > 0.0)
        .unwrap_or(0.0);
    let bit_rate = number(stream.and_then(|v| v.get("bit_rate")))
        .or_else(|| number(format.and_then(|v| v.get("bit_rate"))))
        .filter(|value| *value > 0.0)
        .map(|value| value as u64);

    Ok(AudioProbeResult {
        title: tag(stream, format, "title"),
        artist: tag(stream, format, "artist"),
        album: tag(stream, format, "album"),
        codec: string(stream.and_then(|v| v.get("codec_name"))).unwrap_or_else(|| "unknown".into()),
        sample_rate,
        bit_depth,
        channels,
        bit_rate,
        duration,
    })
}

fn spectrogram_args(
    program: &str,
    input: &Path,
    output: &Path,
    title: Option<&str>,
    comment: Option<&str>,
    duration_secs: f64,
) -> Vec<String> {
    let mut args = vec![
        program.to_owned(),
        input.display().to_string(),
        "-n".into(),
        "spectrogram".into(),
        "-x".into(),
        "1200".into(),
        "-y".into(),
        "551".into(),
        "-z".into(),
        "120".into(),
    ];
    if duration_secs > 0.0 {
        args.extend(["-d".into(), format!("{duration_secs:.2}")]);
    }
    if let Some(title) = title {
        args.extend(["-t".into(), title.to_owned()]);
    }
    if let Some(comment) = comment {
        args.extend(["-c".into(), comment.to_owned()]);
    }
    args.extend(["-o".into(), output.display().to_string()]);
    args
}

fn process_error(prefix: &str, status: Option<std::process::ExitStatus>, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    if stderr.is_empty() {
        match status {
            Some(status) => format!("{prefix} exited with {status}"),
            None => format!("{prefix} failed"),
        }
    } else {
        stderr
    }
}

/// Generate a SoX spectrogram, falling back to an ffmpeg-to-SoX pipe.
pub async fn generate_spectrogram(
    input: &Path,
    output: &Path,
    title: Option<&str>,
    comment: Option<&str>,
    duration_secs: f64,
) -> Result<(), String> {
    let _permit = spectrogram_slot()
        .acquire_owned()
        .await
        .map_err(|_| "spectrogram worker unavailable".to_owned())?;
    let args = spectrogram_args("sox", input, output, title, comment, duration_secs);
    let mut direct_command = Command::new(&args[0]);
    direct_command
        .args(&args[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let direct = run_bounded(direct_command).await;
    if let Ok(result) = direct {
        if result.status.success() && output.exists() {
            return Ok(());
        }
    }

    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(input)
        .args(["-f", "wav", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut ffmpeg_stdout = ffmpeg
        .stdout
        .take()
        .ok_or_else(|| "ffmpeg stdout unavailable".to_owned())?;
    let ffmpeg_stderr = ffmpeg
        .stderr
        .take()
        .ok_or_else(|| "ffmpeg stderr unavailable".to_owned())?;
    let ffmpeg_stderr_task =
        tokio::spawn(async move { read_limited(ffmpeg_stderr).await.unwrap_or_default() });

    let sox_args = spectrogram_args("sox", Path::new("-"), output, title, comment, duration_secs);
    let mut sox = match Command::new(&sox_args[0])
        .args(["-t", "wav"])
        .args(&sox_args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(sox) => sox,
        Err(error) => {
            let _ = ffmpeg.kill().await;
            return Err(error.to_string());
        }
    };
    let mut sox_stdin = sox
        .stdin
        .take()
        .ok_or_else(|| "sox stdin unavailable".to_owned())?;
    let sox_stderr = sox
        .stderr
        .take()
        .ok_or_else(|| "sox stderr unavailable".to_owned())?;
    let sox_stderr_task =
        tokio::spawn(async move { read_limited(sox_stderr).await.unwrap_or_default() });

    let pipeline = async {
        let copy_result = tokio::io::copy(&mut ffmpeg_stdout, &mut sox_stdin).await;
        let _ = sox_stdin.shutdown().await;
        let ffmpeg_status = ffmpeg.wait().await.ok();
        let sox_status = sox.wait().await.ok();
        (copy_result, ffmpeg_status, sox_status)
    };
    let (copy_result, ffmpeg_status, sox_status) =
        match tokio::time::timeout(PROCESS_TIMEOUT, pipeline).await {
            Ok(result) => result,
            Err(_) => {
                let _ = ffmpeg.kill().await;
                let _ = sox.kill().await;
                let _ = ffmpeg.wait().await;
                let _ = sox.wait().await;
                return Err("audio process timed out".to_owned());
            }
        };
    let ffmpeg_error = ffmpeg_stderr_task.await.unwrap_or_default();
    let sox_error = sox_stderr_task.await.unwrap_or_default();

    if let Err(error) = copy_result {
        return Err(error.to_string());
    }
    if ffmpeg_status.is_none_or(|status| !status.success()) {
        return Err(process_error("ffmpeg", ffmpeg_status, &ffmpeg_error));
    }
    if sox_status.is_none_or(|status| !status.success()) || !output.exists() {
        return Err(process_error("sox", sox_status, &sox_error));
    }
    Ok(())
}

/// Format a duration like the TypeScript command (`m:ss` or `h:mm:ss`).
pub fn format_duration(seconds: f64) -> String {
    let mins = (seconds / 60.0).floor() as u64;
    let secs = (seconds % 60.0).floor() as u64;
    if mins >= 60 {
        format!("{}:{:02}:{:02}", mins / 60, mins % 60, secs)
    } else {
        format!("{mins}:{secs:02}")
    }
}

pub fn thousands_separator(value: u32) -> String {
    let digits = value.to_string();
    let first = digits.len() % 3;
    if first == 0 {
        digits
            .as_bytes()
            .chunks(3)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        let mut result = digits[..first].to_owned();
        for chunk in digits.as_bytes()[first..].chunks(3) {
            result.push(',');
            result.push_str(std::str::from_utf8(chunk).unwrap());
        }
        result
    }
}

/// Build the HTML caption sent with the generated image.
pub fn build_caption(probe: &AudioProbeResult, song_title: &str, codec: &str) -> String {
    let mut caption = format!(
        "📊 <b>Audio Spectrogram Analysis</b><br/><br/>• <b>Track:</b> <code>{}</code><br/>",
        crate::html::escape(song_title)
    );
    if let Some(artist) = &probe.artist {
        caption.push_str(&format!(
            "• <b>Artist:</b> <code>{}</code><br/>",
            crate::html::escape(artist)
        ));
    }
    if let Some(album) = &probe.album {
        caption.push_str(&format!(
            "• <b>Album:</b> <code>{}</code><br/>",
            crate::html::escape(album)
        ));
    }
    caption.push_str(&format!(
        "• <b>Codec:</b> <code>{}</code><br/>• <b>Sample Rate:</b> <code>{} Hz ({} kHz)</code><br/>",
        crate::html::escape(codec),
        thousands_separator(probe.sample_rate),
        (f64::from(probe.sample_rate) / 100.0).round() / 10.0
    ));
    if let Some(bit_depth) = probe.bit_depth {
        caption.push_str(&format!(
            "• <b>Bit Depth:</b> <code>{bit_depth}-bit</code><br/>"
        ));
    }
    let channels = match probe.channels {
        1 => "Mono".to_owned(),
        2 => "Stereo (2.0)".to_owned(),
        channels => format!("{channels} channels"),
    };
    caption.push_str(&format!("• <b>Channels:</b> <code>{channels}</code><br/>"));
    if let Some(bit_rate) = probe.bit_rate {
        caption.push_str(&format!(
            "• <b>Bitrate:</b> <code>{} kbps</code><br/>",
            (bit_rate as f64 / 1000.0).round()
        ));
    }
    if probe.duration > 0.0 {
        caption.push_str(&format!(
            "• <b>Duration:</b> <code>{}</code><br/>",
            format_duration(probe.duration)
        ));
    }
    caption
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(65.0), "1:05");
        assert_eq!(format_duration(3610.0), "1:00:10");
        assert_eq!(format_duration(59.0), "0:59");
    }

    #[test]
    fn has_all_audio_extensions() {
        assert!(AUDIO_EXTENSIONS.contains(&".m4a"));
        assert!(AUDIO_EXTENSIONS.contains(&".flac"));
        assert_eq!(AUDIO_EXTENSIONS.len(), 18);
    }

    #[test]
    fn separates_thousands() {
        assert_eq!(thousands_separator(44_100), "44,100");
    }

    #[test]
    fn builds_oracle_caption() {
        let probe = AudioProbeResult {
            title: Some("Song".into()),
            artist: None,
            album: None,
            codec: "ALAC".into(),
            sample_rate: 44_100,
            bit_depth: Some(24),
            channels: 2,
            bit_rate: Some(900_000),
            duration: 180.5,
        };
        let caption = build_caption(&probe, "Song", "ALAC");
        assert!(caption.contains("• <b>Track:</b> <code>Song</code><br/>"));
        assert!(caption.contains("• <b>Codec:</b> <code>ALAC</code><br/>"));
        assert!(caption.contains("• <b>Sample Rate:</b> <code>44,100 Hz (44.1 kHz)</code><br/>"));
        assert!(caption.contains("• <b>Bit Depth:</b> <code>24-bit</code><br/>"));
        assert!(caption.contains("• <b>Channels:</b> <code>Stereo (2.0)</code><br/>"));
        assert!(caption.contains("• <b>Bitrate:</b> <code>900 kbps</code><br/>"));
        assert!(caption.contains("• <b>Duration:</b> <code>3:00</code><br/>"));
    }
}
