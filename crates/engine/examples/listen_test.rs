//! Listening test: rip a real track through every quality path and save
//! each as a separate file so they can be auditioned side by side.
//!
//! For each track ID this downloads:
//!   - `<id>_default_<codec>.m4a` — highest available quality
//!     (FairPlay ALAC when lossless exists, else Widevine CENC AAC)
//!   - `<id>_atmos_<codec>.m4a`  — Dolby Atmos (ec-3) when offered,
//!     otherwise the same highest-quality fallback (skipped as a
//!     duplicate when it matches the default rip's codec)
//!
//! Usage:
//!   cargo run -p engine --example listen_test -- [outdir] [trackId...]
//!
//! Defaults: outdir `/tmp/opencode/listen`, tracks `1499378607`
//! (ALAC + Atmos capable) and `1561413895` (no lossless → CENC AAC).
//! Wrapper URL comes from `ALAC_WRAPPER_URL` (default
//! `http://localhost:12340`).

use engine::{
    streaming::StreamError,
    wrapper::{CodecPreference, WrapperEngine},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "bot-data/downloads".into());
    let track_ids: Vec<String> = {
        let ids: Vec<String> = args.collect();
        if ids.is_empty() {
            vec![
                "1499378607".to_owned(), // lossless + Atmos (ec-3)
                "1561413895".to_owned(), // no lossless → CENC AAC
            ]
        } else {
            ids
        }
    };

    let wrapper_url =
        std::env::var("ALAC_WRAPPER_URL").unwrap_or_else(|_| "http://localhost:12340".into());
    let engine = WrapperEngine::new(&wrapper_url, None);
    std::fs::create_dir_all(&out_dir)?;

    println!("wrapper: {wrapper_url}");
    println!("output:  {out_dir}\n");

    let mut results: Vec<String> = Vec::new();

    for track_id in &track_ids {
        let default = rip_one(
            &engine,
            track_id,
            CodecPreference::HighestQuality,
            "default",
            &out_dir,
        )
        .await;
        match default {
            Ok(Some((path, codec))) => {
                results.push(format!("{track_id}  default  {codec:<10} {path}"));
                // Atmos preference only produces a new file when the track
                // actually offers an ec-3 variant; otherwise the fallback
                // duplicates the default rip.
                if codec != "ec-3" {
                    match rip_one(&engine, track_id, CodecPreference::Atmos, "atmos", &out_dir)
                        .await
                    {
                        Ok(Some((atmos_path, atmos_codec))) => {
                            // A track with no Atmos variant falls back to the
                            // same stream the default rip produced (equal
                            // codec); drop the duplicate instead of keeping
                            // two identical files.
                            if atmos_codec == codec {
                                let _ = std::fs::remove_file(&atmos_path);
                            } else {
                                results.push(format!(
                                    "{track_id}  atmos   {atmos_codec:<10} {atmos_path}"
                                ));
                            }
                        }
                        Ok(None) => {}
                        Err(e) => results.push(format!("{track_id}  atmos   FAILED     {e}")),
                    }
                }
            }
            Ok(None) => {}
            Err(e) => results.push(format!("{track_id}  default  FAILED     {e}")),
        }
    }

    println!("\n=== listening test files ===");
    for line in &results {
        println!("{line}");
    }
    println!("\nlisten with: ffplay <file>   (or any player)");
    Ok(())
}

/// Rip one preference and write it out. `Ok(None)` means the stream was
/// empty (nothing written).
async fn rip_one(
    engine: &WrapperEngine,
    track_id: &str,
    preference: CodecPreference,
    label: &str,
    out_dir: &str,
) -> Result<Option<(String, String)>, StreamError> {
    println!("[{track_id}] ripping {label}...");
    let progress = std::sync::Arc::new(|msg: &str| println!("    {msg}"))
        as std::sync::Arc<dyn Fn(&str) + Send + Sync>;

    let source = engine
        .rip_track(track_id, None, Some(progress), preference)
        .await?;

    let mut collected = Vec::new();
    use futures_util::StreamExt;
    let mut stream = source.stream;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| StreamError::Message(e.to_string()))?;
        collected.extend_from_slice(&chunk);
    }
    if collected.len() < 8 {
        eprintln!("[{track_id}] {label}: empty stream");
        return Ok(None);
    }

    let codec = source.codec.clone();
    let path = format!("{out_dir}/{track_id}_{label}_{codec}.m4a");
    std::fs::write(&path, &collected).map_err(|e| StreamError::Message(e.to_string()))?;

    let mins = collected.len() as f64 / 1024.0 / 1024.0;
    println!(
        "[{track_id}] {label}: {codec} {}Hz {}-bit {mins:.1}MB -> {path}",
        source.sample_rate, source.bit_depth
    );
    Ok(Some((path, codec)))
}
