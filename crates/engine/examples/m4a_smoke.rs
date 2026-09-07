// One-off live smoke for tag_m4a_file with the real ffmpeg (not part of suite).
use std::path::Path;

use engine::tagger::{tag_m4a_file, ProcessRunner};
use engine::types::TrackMeta;

#[tokio::main]
async fn main() {
    let dir = std::path::PathBuf::from("/tmp/opencode/tag-smoke");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = dir.join("in.raw");
    let out = dir.join("out.m4a");

    // Stage: 2s of silence encoded as ALAC inside a bare M4A (no tags yet).
    let stage = dir.join("stage.m4a");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y", "-f", "lavfi", "-i", "anullsrc=r=96000:cl=mono",
            "-t", "2", "-c:a", "alac", stage.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "staging failed");
    let _ = &raw;

    let meta = TrackMeta {
        id: "1".into(),
        title: "Smoke Title".into(),
        artist: "Smoke Artist".into(),
        album: "Smoke Album".into(),
        album_artist: "Smoke Artist".into(),
        genre: Some("Pop".into()),
        release_date: "2024-05-06".into(),
        composer: None,
        track_number: Some(3),
        track_count: Some(9),
        disc_number: Some(1),
        disc_count: Some(1),
        duration_secs: 2,
        explicit: false,
        artwork_url: String::new(),
    };
    // Real 64x64 JPEG cover generated via ffmpeg.
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y", "-v", "quiet", "-f", "lavfi", "-i", "color=c=red:s=64x64",
            "-frames:v", "1", dir.join("cover.jpg").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let cover: Vec<u8> = std::fs::read(dir.join("cover.jpg")).unwrap();
    let result = tag_m4a_file(&ProcessRunner, &stage, &out, &meta, Some(&cover), Some("la\nla")).await;
    match result {
        Ok(path) => println!("tagged: {}", path.display()),
        Err(e) => { println!("ERR: {e}"); std::process::exit(1); }
    }
    let _ = Path::new(&out).canonicalize().unwrap();
}
