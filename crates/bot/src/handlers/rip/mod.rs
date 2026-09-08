//! Live `/alac` command policy and pipeline entry point.

#[allow(dead_code)]
pub mod cancel;
#[allow(dead_code)]
pub mod gates;
#[allow(dead_code)]
pub mod input;
#[allow(dead_code)]
pub mod status;

use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[allow(unused_imports)]
pub use cancel::{ActiveJobs, RipJobHandle};
use ferogram::{
    filters::{self, Dispatcher},
    InputMessage,
};

use crate::BotState;

static ACTIVE_JOBS: OnceLock<ActiveJobs> = OnceLock::new();

pub fn active_jobs() -> ActiveJobs {
    ACTIVE_JOBS.get_or_init(cancel::new_jobs).clone()
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    for alias in [
        "alac", "rip", "batch", "dl", "download", "rerip", "cache", "dump",
    ] {
        let state = Arc::clone(&state);
        dp.on_message(filters::command(alias), move |msg| {
            let state = Arc::clone(&state);
            async move { handle_command(state, msg).await }
        });
    }
    let state = Arc::clone(&state);
    dp.on_message(filters::command("cancel"), move |msg| {
        let state = Arc::clone(&state);
        async move { handle_cancel_command(state, msg).await }
    });
}

async fn handle_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    let chat = msg.chat_id();
    if !state
        .auth
        .is_authorized(sender, Some(chat))
        .await
        .unwrap_or(false)
    {
        return;
    }
    let command = input::command_name(msg.text().unwrap_or_default()).unwrap_or_default();
    let is_cache = command == "cache" || command == "dump";
    let admin = state.auth.is_admin(sender);
    if let Some(text) = gates::cache_gate(is_cache, admin) {
        reply(&msg, text).await;
        return;
    }
    let parsed = input::parse_message(&state.client, &msg, command == "rerip").await;
    if parsed.items.is_empty() {
        reply(&msg, gates::usage(is_cache)).await;
        return;
    }
    if let Some(text) = gates::force_gate(parsed.force, admin) {
        reply(&msg, text).await;
        return;
    }
    let status_sink: Arc<dyn status::StatusSink> = Arc::new(status::TelegramStatusSink {
        client: state.client.clone(),
        peer: ferogram::PeerRef::from(chat),
    });
    let status_message_id = match status_sink.send(status::initial_text(), None).await {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(%error, "failed to send rip resolving status");
            return;
        }
    };
    let job_id = format!("job_{}", now_ms());
    let status = Arc::new(status::StatusEditor::new(
        status_sink,
        status_message_id,
        job_id,
    ));
    let options = RipOptions {
        chat_id: chat,
        user_id: sender,
        user_name: "User".to_owned(),
        is_group: chat != sender,
        cache_only: is_cache,
        force: parsed.force,
        is_admin: admin,
        storefront: parsed.storefront,
        items: parsed.items,
        reply_to: Some(i64::from(msg.id())),
    };
    if let Err(error) = start_rip_job(
        Arc::clone(&state.rip_deps),
        state.rip_queue.clone(),
        options,
        status,
    )
    .await
    {
        tracing::error!(%error, "rip pipeline failed");
    }
}

async fn handle_cancel_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let caller = msg.sender_user_id().unwrap_or_default();
    let admin = state.auth.is_admin(caller);
    let name = if admin { "Admin" } else { "User" };
    if cancel::cancel_command(&active_jobs(), msg.chat_id(), caller, admin, name)
        .await
        .is_some()
    {
        reply(&msg, cancel::COMMAND_ACK).await;
    } else {
        reply(&msg, cancel::NO_ACTIVE).await;
    }
}

async fn reply(msg: &ferogram::update::IncomingMessage, text: &str) {
    let _ = msg.reply(InputMessage::html(text)).await;
}

/// Pipeline options shared by tests and by the M5b integration hook.
#[derive(Debug, Clone)]
pub struct RipOptions {
    pub chat_id: i64,
    pub user_id: i64,
    pub user_name: String,
    pub is_group: bool,
    pub cache_only: bool,
    pub force: bool,
    pub is_admin: bool,
    pub storefront: Option<String>,
    pub items: Vec<engine::types::ParsedTargetItem>,
    pub reply_to: Option<i64>,
}

/// Generic seam used by offline tests and by the integration lane. It keeps
/// the dependency boundary explicit without calling `RipOrchestrator`: the
/// live command's maintenance timing and status lifecycle differ from it.
pub async fn start_rip_job<D: engine::orchestrator::deps::OrchestratorDeps>(
    deps: Arc<D>,
    queue: engine::queue::SequentialRipQueue,
    options: RipOptions,
    status: Arc<status::StatusEditor>,
) -> Result<(), engine::orchestrator::OrchestratorError> {
    use engine::{
        orchestrator::{
            caption::{format_dump_caption, html_escape, DumpCaptionMetadata},
            deps::{RequestLog, SaveTrackInput},
        },
        queue::EnqueueOptions,
        types::TargetKind,
    };
    let settings = deps.get_settings().await;
    if !options.cache_only && !settings.can_rip_live(options.is_admin) {
        return Err(engine::orchestrator::OrchestratorError::Message("Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now.".to_owned()));
    }
    let storefront = options
        .storefront
        .clone()
        .unwrap_or_else(|| "us".to_owned());
    #[derive(Clone)]
    struct Target {
        id: String,
        title: Option<String>,
        artist: Option<String>,
        storefront: String,
    }
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut header = String::new();
    let mut resolution_failures = Vec::new();
    for item in &options.items {
        if item.kind == TargetKind::Track {
            if seen.insert(item.id.clone()) {
                targets.push(Target {
                    id: item.id.clone(),
                    title: None,
                    artist: None,
                    storefront: item
                        .storefront
                        .clone()
                        .unwrap_or_else(|| storefront.clone()),
                });
            }
            continue;
        }
        let sf = item
            .storefront
            .clone()
            .unwrap_or_else(|| storefront.clone());
        let resolved: Result<(String, Vec<Target>), String> = match item.kind {
            TargetKind::Album => deps.fetch_album_tracks(&item.id, &sf).await.map(|a| {
                (
                    format!(
                        "Album: <b>{}</b> by <b>{}</b>",
                        html_escape(&a.album.album),
                        html_escape(&a.album.artist)
                    ),
                    a.tracks
                        .into_iter()
                        .map(|t| Target {
                            id: t.id,
                            title: Some(t.title),
                            artist: Some(t.artist),
                            storefront: sf.clone(),
                        })
                        .collect(),
                )
            }),
            TargetKind::Artist => deps.fetch_artist_tracks(&item.id, &sf).await.map(|a| {
                (
                    format!(
                        "Artist: <b>{}</b> (Discography)",
                        html_escape(&a.artist_name)
                    ),
                    a.tracks
                        .into_iter()
                        .map(|t| Target {
                            id: t.id,
                            title: Some(t.title),
                            artist: Some(t.artist),
                            storefront: sf.clone(),
                        })
                        .collect(),
                )
            }),
            TargetKind::Playlist => deps
                .fetch_playlist_tracks(&item.id, &sf)
                .await
                .map(|p| {
                    let curator = p
                        .curator_name
                        .map(|n| format!(" ({})", html_escape(&n)))
                        .unwrap_or_default();
                    (
                        format!("Playlist: <b>{}</b>{curator}", html_escape(&p.title)),
                        p.tracks
                            .into_iter()
                            .map(|t| Target {
                                id: t.id,
                                title: Some(t.title),
                                artist: Some(t.artist),
                                storefront: sf.clone(),
                            })
                            .collect(),
                    )
                })
                .map_err(|e| e.to_string()),
            TargetKind::Track => unreachable!(),
        };
        match resolved {
            Ok((new_header, resolved)) => {
                if header.is_empty() {
                    header = new_header;
                }
                for target in resolved {
                    if seen.insert(target.id.clone()) {
                        targets.push(target);
                    }
                }
            }
            Err(error) => resolution_failures.push(format!(
                "{} {}: {}",
                format_kind(item.kind),
                item.id,
                error
            )),
        }
    }
    if targets.is_empty() {
        let detail = if resolution_failures.is_empty() {
            String::new()
        } else {
            format!(
                "<br/><code>{}</code>",
                html_escape(&resolution_failures.join("\n"))
            )
        };
        status
            .final_text(format!("⚠️ <b>Failed to resolve any tracks:</b>{detail}"))
            .await;
        return Ok(());
    }
    let original = targets.len();
    let capped = if !options.is_admin && settings.max_collection_tracks > 0 {
        original.saturating_sub(settings.max_collection_tracks as usize)
    } else {
        0
    };
    if capped > 0 {
        targets.truncate(settings.max_collection_tracks as usize);
    }
    if header.is_empty() {
        header = if options.cache_only {
            if targets.len() > 1 {
                format!("Batch Cache: <b>{} tracks</b>", targets.len())
            } else {
                format!("Track Cache: <code>{}</code>", targets[0].id)
            }
        } else if targets.len() > 1 {
            format!("Batch: <b>{} tracks</b>", targets.len())
        } else {
            format!("Track ID: <code>{}</code>", targets[0].id)
        };
    }
    let ids: Vec<String> = targets.iter().map(|t| t.id.clone()).collect();
    let cached = deps
        .find_cached_tracks(&ids)
        .await
        .map_err(engine::orchestrator::OrchestratorError::Message)?;
    if options.force {
        let old_messages: Vec<i64> = cached
            .values()
            .map(|row| row.message_id)
            .filter(|id| *id > 0)
            .collect();
        if !old_messages.is_empty() {
            let _ = deps.sink().delete_dump_messages(&old_messages).await;
        }
        for id in &ids {
            let _ = deps.delete_track(id).await;
        }
    }
    let mut state = status::ProgressState {
        header: header.clone(),
        total: targets.len(),
        cache_only: options.cache_only,
        group: options.is_group,
        ..Default::default()
    };
    let is_multi_track = targets.len() > 1;
    let mut uncached = Vec::new();
    for target in &targets {
        if !options.force && cached.contains_key(&target.id) {
            if !options.cache_only {
                let row = &cached[&target.id];
                if deps
                    .sink()
                    .send_dump_copy(
                        options.chat_id,
                        row.message_id,
                        options.reply_to,
                        is_multi_track,
                    )
                    .await
                    .is_err()
                {
                    // TS: a cache copy failure falls back to re-ripping.
                    uncached.push(target.clone());
                    continue;
                }
            }
            state.cached += 1;
        } else {
            uncached.push(target.clone());
        }
    }
    status
        .update(
            status::render_progress(
                &state,
                Some(&format!(
                    "⚡ {} cached • ⏳ {} track{} waiting in queue...",
                    state.cached,
                    uncached.len(),
                    if uncached.len() == 1 { "" } else { "s" }
                )),
            ),
            true,
            false,
        )
        .await;
    let job_id = format!("{}_{}", now_ms(), status.job_id());
    let controller = tokio_util::sync::CancellationToken::new();
    let handle = RipJobHandle {
        id: job_id.clone(),
        chat_id: options.chat_id,
        requester_id: options.user_id,
        target: header.clone(),
        total: targets.len(),
        processed: Arc::new(tokio::sync::Mutex::new(state.cached)),
        controller: controller.clone(),
        status: status.clone(),
        completed: Arc::new(tokio::sync::Mutex::new(false)),
    };
    active_jobs()
        .lock()
        .await
        .insert(job_id.clone(), handle.clone());
    let deps_for_queue = Arc::clone(&deps);
    let options_for_queue = options.clone();
    let status_for_queue = status.clone();
    let state_for_queue = Arc::new(tokio::sync::Mutex::new(state));
    let processed = handle.processed.clone();
    let uncached_for_queue = uncached.clone();
    let header_for_queue = header.clone();
    let controller_for_task = controller.clone();
    let job_id_for_task = job_id.clone();
    let position_status = status.clone();
    let position_header = header.clone();
    let position_cached = state_for_queue.lock().await.cached;
    let queue_result = queue
        .enqueue(
            move |task_signal| {
                Box::pin(async move {
                    let mut failed = Vec::new();
                    let temp = std::env::temp_dir().join(format!("alac_job_{}", job_id_for_task));
                    let _ = tokio::fs::create_dir_all(&temp).await;
                    for target in uncached_for_queue {
                        if task_signal.is_cancelled() || controller_for_task.is_cancelled() {
                            break;
                        }
                        let process_no = *processed.lock().await;
                        let item_title = target
                            .title
                            .as_ref()
                            .map(|t| {
                                format!("{} - {}", target.artist.as_deref().unwrap_or("Unknown"), t)
                            })
                            .unwrap_or_else(|| format!("Track #{}", process_no + 1));
                        {
                            let mut s = state_for_queue.lock().await;
                            s.active_download = Some(format!(
                                "📥 <b>Downloading:</b> {}",
                                html_escape(&item_title)
                            ));
                            let text = status::render_progress(&s, None);
                            status_for_queue.update(text, false, false).await;
                        }
                        let download_status = status_for_queue.clone();
                        let download_state = state_for_queue.clone();
                        let download_title = item_title.clone();
                        let progress_callback: engine::ripper::RipProgressCallback =
                            Arc::new(move |status_text, downloaded, total| {
                                let line = if let (Some(current), Some(total)) =
                                    (downloaded, total.filter(|n| *n > 0))
                                {
                                    format!(
                                        "📥 <b>Downloading:</b> {} <code>[{:.1}/{:.1} MB]</code>",
                                        html_escape(&download_title),
                                        current as f64 / 1_048_576.0,
                                        total as f64 / 1_048_576.0
                                    )
                                } else if status_text.contains("Tagging") {
                                    format!("🏷️ <b>Tagging:</b> {}", html_escape(&download_title))
                                } else {
                                    format!(
                                        "📥 <b>Downloading:</b> {}",
                                        html_escape(&download_title)
                                    )
                                };
                                let status_status = download_status.clone();
                                let status_state = download_state.clone();
                                tokio::spawn(async move {
                                    let mut state = status_state.lock().await;
                                    state.active_download = Some(line);
                                    let text = status::render_progress(&state, None);
                                    let _ = status_status.update(text, false, false).await;
                                });
                            });
                        let rip = deps_for_queue
                            .rip(
                                &target.id,
                                Some(&progress_callback),
                                &target.storefront,
                                task_signal.clone(),
                                Some(&temp),
                            )
                            .await;
                        let rip = match rip {
                            Ok(r) => r,
                            Err(error) => {
                                let msg = error.to_string();
                                failed.push((target.id.clone(), msg.clone()));
                                let mut s = state_for_queue.lock().await;
                                s.failed += 1;
                                s.active_download = None;
                                let _ = status_for_queue
                                    .update(status::render_progress(&s, None), false, false)
                                    .await;
                                if is_mirror_down(&msg) {
                                    break;
                                }
                                continue;
                            }
                        };
                        {
                            let mut s = state_for_queue.lock().await;
                            s.active_download = None;
                            s.active_upload =
                                Some(format!("📤 <b>Uploading:</b> {}", html_escape(&item_title)));
                            let _ = status_for_queue
                                .update(status::render_progress(&s, None), false, false)
                                .await;
                        }
                        let caption = format_dump_caption(&DumpCaptionMetadata::from((
                            &rip,
                            target.id.as_str(),
                        )));
                        let upload_status = status_for_queue.clone();
                        let upload_state = state_for_queue.clone();
                        let upload_title = item_title.clone();
                        let upload_progress: engine::orchestrator::deps::UploadProgressCallback =
                            Arc::new(move |current, total| {
                                if total == 0 {
                                    return;
                                }
                                let line = format!(
                                    "📤 <b>Uploading:</b> {} <code>[{:.1}/{:.1} MB]</code>",
                                    html_escape(&upload_title),
                                    current as f64 / 1_048_576.0,
                                    total as f64 / 1_048_576.0
                                );
                                let status_status = upload_status.clone();
                                let status_state = upload_state.clone();
                                tokio::spawn(async move {
                                    let mut state = status_state.lock().await;
                                    state.active_upload = Some(line);
                                    let text = status::render_progress(&state, None);
                                    let _ = status_status.update(text, false, false).await;
                                });
                            });
                        let max_retries = std::env::var("ALAC_MAX_RETRIES")
                            .ok()
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(3);
                        let mut upload = Err(engine::orchestrator::deps::SinkError(
                            "upload failed".to_owned(),
                        ));
                        for attempt in 0..=max_retries {
                            upload = deps_for_queue
                                .sink()
                                .send_audio_to_dump(
                                    &rip.file_path,
                                    &rip.title,
                                    &rip.artist,
                                    rip.duration,
                                    &caption,
                                    Some(&upload_progress),
                                )
                                .await;
                            if upload.is_ok() {
                                break;
                            }
                            if attempt < max_retries {
                                let raw = deps_for_queue.upload_retry_base_ms() * 2u64.pow(attempt);
                                let jitter = 800 + (now_ms() % 401) as u64;
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    raw * jitter / 1000,
                                ))
                                .await;
                            }
                        }
                        match upload {
                            Ok(Some(dump)) => {
                                let _ = deps_for_queue
                                    .save_track(SaveTrackInput::from_rip_result(
                                        &target.id,
                                        &rip,
                                        dump.message_id,
                                        &dump.file_id,
                                        &dump.file_unique_id,
                                    ))
                                    .await;
                                let _ = deps_for_queue
                                    .log_request(RequestLog {
                                        telegram_id: options_for_queue.user_id,
                                        chat_id: options_for_queue.chat_id,
                                        apple_track_id: target.id.clone(),
                                        is_cache_hit: false,
                                        duration_ms: None,
                                        status: "completed".to_owned(),
                                        error_reason: None,
                                    })
                                    .await;
                                if !options_for_queue.cache_only {
                                    let _ = deps_for_queue
                                        .sink()
                                        .send_dump_copy(
                                            options_for_queue.chat_id,
                                            dump.message_id,
                                            options_for_queue.reply_to,
                                            is_multi_track,
                                        )
                                        .await;
                                }
                                let mut s = state_for_queue.lock().await;
                                s.ripped += 1;
                                s.active_upload = None;
                                *processed.lock().await += 1;
                                let _ = status_for_queue
                                    .update(status::render_progress(&s, None), false, false)
                                    .await;
                            }
                            Ok(None) | Err(_) => {
                                failed.push((target.id.clone(), "Upload failed".to_owned()));
                                let mut s = state_for_queue.lock().await;
                                s.failed += 1;
                                s.active_upload = None;
                                let _ = status_for_queue
                                    .update(status::render_progress(&s, None), false, false)
                                    .await;
                            }
                        }
                        let _ = tokio::fs::remove_file(&rip.file_path).await;
                    }
                    if controller_for_task.is_cancelled() || task_signal.is_cancelled() {
                        return Ok::<(), ()>(());
                    }
                    let s = state_for_queue.lock().await.clone();
                    let summary = status::SummaryInput {
                        target: header_for_queue,
                        total: s.total,
                        cached: s.cached,
                        ripped: s.ripped,
                        skipped: 0,
                        failed,
                        elapsed: "0.0".to_owned(),
                        cache_only: s.cache_only,
                        group: s.group,
                        capped,
                        cap_limit: settings.max_collection_tracks,
                    };
                    status_for_queue
                        .final_text(status::final_summary(&summary))
                        .await;
                    Ok::<(), ()>(())
                })
            },
            Some(EnqueueOptions {
                signal: Some(controller),
                on_position_change: Some(Arc::new(move |position| {
                    let status = position_status.clone();
                    let header = position_header.clone();
                    tokio::spawn(async move {
                        let state = status::ProgressState {
                            header,
                            total: 1,
                            cached: position_cached,
                            ..Default::default()
                        };
                        let text = status::render_progress(
                            &state,
                            Some(&format!(
                                "⚡ {} cached • ⏳ In Queue: Position #{}",
                                position_cached, position
                            )),
                        );
                        let _ = status.update(text, true, false).await;
                    });
                })),
                on_start: Some(Arc::new(|| {})),
            }),
        )
        .await;
    active_jobs().lock().await.remove(&job_id);
    queue_result
        .map_err(|e| engine::orchestrator::OrchestratorError::Message(e.to_string()))?
        .map_err(|_| {
            engine::orchestrator::OrchestratorError::Message("Rip pipeline failed.".to_owned())
        })
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn format_kind(kind: engine::types::TargetKind) -> &'static str {
    match kind {
        engine::types::TargetKind::Track => "Track",
        engine::types::TargetKind::Album => "Album",
        engine::types::TargetKind::Playlist => "Playlist",
        engine::types::TargetKind::Artist => "Artist",
    }
}
pub fn is_mirror_down(error: &str) -> bool {
    [
        "Mirror /status check timed out",
        "Mirror health check failed",
        "Lossless wrapper is currently offline",
        "Mirror manifest lookup timed out",
        "Mirror service is currently offline",
        "Failed to connect to mirror stream",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}
