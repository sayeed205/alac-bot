//! Tagger parity tests against `src/modules/alac/tagger.ts`.

use std::{path::Path, sync::Mutex};

use engine::{
    tagger::{build_track_filename, sanitize_filename, tag_m4a_file, FfmpegRunner},
    types::TrackMeta,
};

fn meta() -> TrackMeta {
    TrackMeta {
        id: "1".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        album_artist: "Artist".into(),
        genre: Some("Pop".into()),
        release_date: "2020-01-02".into(),
        composer: None,
        track_number: Some(3),
        track_count: Some(12),
        disc_number: Some(1),
        disc_count: Some(2),
        duration_secs: 100,
        explicit: false,
        artwork_url: String::new(),
    }
}

#[test]
fn sanitize_replaces_and_defaults() {
    assert_eq!(
        sanitize_filename("a<b>c:d\"e/f\\g|h?i*j"),
        "a_b_c_d_e_f_g_h_i_j"
    );
    assert_eq!(sanitize_filename("   "), "track");
    assert_eq!(sanitize_filename("  ok  "), "ok");
}

#[test]
fn filename_explicit_and_number_padding() {
    let mut m = meta();
    m.explicit = true;
    assert_eq!(build_track_filename(&m), "03. Song - Artist [E] [ALAC].m4a");
    m.explicit = false;
    m.track_number = Some(0); // JS falsy → 1
    assert_eq!(build_track_filename(&m), "01. Song - Artist [ALAC].m4a");
    m.track_number = Some(123); // 3-digit: padStart(2) keeps it as-is
    assert_eq!(build_track_filename(&m), "123. Song - Artist [ALAC].m4a");
}

/// Records args; scripts the exit.
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
}

impl FfmpegRunner for FakeRunner {
    async fn run(&self, args: &[String]) -> Result<(), (i32, String)> {
        self.calls.lock().unwrap().push(args.to_vec());
        Ok(())
    }
}

struct FailingRunner;
impl FfmpegRunner for FailingRunner {
    async fn run(&self, _args: &[String]) -> Result<(), (i32, String)> {
        Err((1, "x".repeat(300)))
    }
}

#[tokio::test]
async fn ffmpeg_args_with_cover() {
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw.bin");
    std::fs::write(&raw, b"raw").unwrap();
    let out = dir.path().join("out.m4a");
    tag_m4a_file(&runner, &raw, &out, &meta(), Some(&[1, 2, 3]), None)
        .await
        .unwrap();
    let args = runner.calls.lock().unwrap()[0].clone();
    let cover_arg = format!("{}.cover.jpg", out.display());
    let expected: Vec<String> = [
        "ffmpeg".to_owned(),
        "-y".to_owned(),
        "-i".to_owned(),
        raw.to_str().unwrap().to_owned(),
        "-i".to_owned(),
        cover_arg,
        "-map".to_owned(),
        "0:a".to_owned(),
        "-map".to_owned(),
        "1".to_owned(),
        "-c".to_owned(),
        "copy".to_owned(),
        "-disposition:v:0".to_owned(),
        "attached_pic".to_owned(),
    ]
    .into_iter()
    .chain(metadata_args(&meta(), None))
    .chain([out.to_str().unwrap().to_owned()])
    .collect();
    assert_eq!(args, expected);
    assert!(
        !out.with_file_name("out.m4a.cover.jpg").exists(),
        "cover temp removed"
    );
}

#[tokio::test]
async fn ffmpeg_args_without_cover() {
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw.bin");
    let out = dir.path().join("out.m4a");
    tag_m4a_file(&runner, &raw, &out, &meta(), None, Some("lala"))
        .await
        .unwrap();
    let args = runner.calls.lock().unwrap()[0].clone();
    assert!(!args.contains(&"-map".to_owned()));
    assert!(args.windows(2).any(|w| w == ["-c", "copy"]));
    assert!(args.windows(2).any(|w| w == ["-metadata", "lyrics=lala"]));
    assert!(!Path::new("out.m4a.cover.jpg").exists());
}

fn metadata_args(meta: &TrackMeta, lyrics: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    let push = |args: &mut Vec<String>, k: &str, v: &str| {
        args.push("-metadata".into());
        args.push(format!("{k}={v}"));
    };
    push(&mut args, "title", &meta.title);
    push(&mut args, "artist", &meta.artist);
    push(&mut args, "album", &meta.album);
    push(&mut args, "album_artist", &meta.album_artist);
    push(&mut args, "date", &meta.release_date);
    if let Some(genre) = &meta.genre {
        push(&mut args, "genre", genre);
    }
    if let Some(composer) = &meta.composer {
        push(&mut args, "composer", composer);
    }
    push(
        &mut args,
        "track",
        &format!(
            "{}/{}",
            meta.track_number.unwrap(),
            meta.track_count.unwrap()
        ),
    );
    push(
        &mut args,
        "disc",
        &format!("{}/{}", meta.disc_number.unwrap(), meta.disc_count.unwrap()),
    );
    if let Some(lyrics) = lyrics {
        push(&mut args, "lyrics", lyrics);
    }
    args
}

#[tokio::test]
async fn js_falsy_metadata_skips() {
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
    let dir = tempfile::tempdir().unwrap();
    let mut m = meta();
    m.title = String::new();
    m.track_number = Some(0); // falsy → no track tag
    m.track_count = Some(0); // falsy → no /count even when number present
    m.disc_number = None;
    m.genre = Some(String::new());
    let raw = dir.path().join("r");
    let out = dir.path().join("o.m4a");
    tag_m4a_file(&runner, &raw, &out, &m, None, None)
        .await
        .unwrap();
    let args = runner.calls.lock().unwrap()[0].clone();
    assert!(!args
        .windows(2)
        .any(|w| w[0] == "-metadata" && w[1].starts_with("title=")));
    assert!(!args
        .windows(2)
        .any(|w| w[0] == "-metadata" && w[1].starts_with("track=")));
    assert!(!args
        .windows(2)
        .any(|w| w[0] == "-metadata" && w[1].starts_with("disc=")));
    assert!(!args
        .windows(2)
        .any(|w| w[0] == "-metadata" && w[1].starts_with("genre=")));
}

#[tokio::test]
async fn track_count_zero_omits_slash() {
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
    let dir = tempfile::tempdir().unwrap();
    let mut m = meta();
    m.track_number = Some(7);
    m.track_count = Some(0);
    let raw = dir.path().join("r");
    let out = dir.path().join("o.m4a");
    tag_m4a_file(&runner, &raw, &out, &m, None, None)
        .await
        .unwrap();
    let args = runner.calls.lock().unwrap()[0].clone();
    assert!(args.windows(2).any(|w| w == ["-metadata", "track=7"]));
    assert!(!args.iter().any(|a| a == "track=7/0"));
}

#[tokio::test]
async fn failure_message_slices_last_200_chars() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("r");
    let out = dir.path().join("o.m4a");
    let error = tag_m4a_file(&FailingRunner, &raw, &out, &meta(), None, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("FFmpeg tagging failed (exit code 1): {}", "x".repeat(200))
    );
}
