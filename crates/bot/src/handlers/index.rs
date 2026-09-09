use std::{
    cmp::max,
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use engine::{
    orchestrator::{caption::parse_dump_caption, deps::SaveTrackInput},
    TrackKey,
};
use ferogram::{
    filters::{self, Dispatcher},
    media::Document,
    InputMessage,
};

use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

static INDEXING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexSummary {
    scanned: u64,
    synced: u64,
    pruned: u64,
    skipped: u64,
    duration_ms: u128,
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("index"), move |msg| {
        let state = Arc::clone(&state);
        async move { handle_index(state, msg).await }
    });
}

async fn handle_index(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    if !state.auth.is_admin(sender) {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(
                "🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.",
            )))
            .await;
        return;
    }

    if INDEXING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(
                "⚠️ <b>Dump channel sync is already in progress.</b>",
            )))
            .await;
        return;
    }

    let result = run_index(state, msg).await;
    INDEXING.store(false, Ordering::Release);
    result
}

async fn run_index(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let status = msg
        .reply(InputMessage::html(parse_dynamic_html(
            "🔄 <b>Initializing Dump Channel Sync...</b>",
        )))
        .await;

    let Ok(status_msg) = status else {
        let error = status
            .err()
            .map(|error| escape(&error.to_string()))
            .unwrap_or_else(|| "Unknown error".to_owned());
        let text = format!("❌ <b>Indexing failed:</b> <code>{error}</code>");
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(&text)))
            .await;
        return;
    };

    let peer = super::chat_peer_ref(&msg);
    let status_id = status_msg.id();
    let last_update = Arc::new(Mutex::new(Instant::now()));
    let progress_client = state.client.clone();
    let progress_peer = peer.clone();
    let progress_last_update = Arc::clone(&last_update);
    let on_progress = move |scanned: u64, synced: u64| {
        let now = Instant::now();
        let should_update = progress_last_update
            .lock()
            .map(|mut last| {
                if now.duration_since(*last).as_millis() >= 2000 {
                    *last = now;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if should_update {
            let client = progress_client.clone();
            let peer = progress_peer.clone();
            let text = format!(
                "🔄 <b>Syncing with Dump Channel...</b><br/><br/>\
                 • Scanned: <code>{scanned}</code> messages<br/>\
                 • Synced: <code>{synced}</code> tracks"
            );
            tokio::spawn(async move {
                let _ = client
                    .edit_message(
                        peer,
                        status_id,
                        InputMessage::html(parse_dynamic_html(&text)),
                    )
                    .await;
            });
        }
    };

    let result = index_dump_channel(&state, &on_progress).await;
    match result {
        Ok(summary) => {
            let text = format_index_summary(&summary);
            if state
                .client
                .edit_message(
                    peer.clone(),
                    status_id,
                    InputMessage::html(parse_dynamic_html(&text)),
                )
                .await
                .is_err()
            {
                let _ = msg
                    .reply(InputMessage::html(parse_dynamic_html(&text)))
                    .await;
            }
        }
        Err(error) => {
            let text = format!("❌ <b>Indexing failed:</b> <code>{}</code>", escape(&error));
            if state
                .client
                .edit_message(
                    peer,
                    status_id,
                    InputMessage::html(parse_dynamic_html(&text)),
                )
                .await
                .is_err()
            {
                let _ = msg
                    .reply(InputMessage::html(parse_dynamic_html(&text)))
                    .await;
            }
        }
    }
}

async fn index_dump_channel(
    state: &BotState,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<IndexSummary, String> {
    let started = Instant::now();
    let mut scanned = 0;
    let mut synced = 0;
    let mut skipped = 0;
    let mut valid_track_ids = HashSet::new();

    let probe = state
        .client
        .send_message(
            state.dump_peer.clone(),
            InputMessage::text("🔄 Indexing..."),
        )
        .await
        .map_err(|error| error.to_string())?;
    let max_id = probe.id();
    if let Ok(messages) = state
        .client
        .get_messages(state.dump_peer.clone(), &[max_id])
        .await
    {
        if let Some(message) = messages.first() {
            let _ = message.delete().await;
        }
    }

    let mut end = max_id - 1;
    while end >= 1 {
        let ids = batch_ids(end, 100);
        if let Ok(messages) = state
            .client
            .get_messages(state.dump_peer.clone(), &ids)
            .await
        {
            for message in messages {
                scanned += 1;
                let Some(document) = message.media().and_then(Document::from_media) else {
                    skipped += 1;
                    continue;
                };
                let Some(audio) =
                    document
                        .raw
                        .attributes
                        .iter()
                        .find_map(|attribute| match attribute {
                            ferogram::tl::enums::DocumentAttribute::Audio(audio)
                                if !audio.voice =>
                            {
                                Some(audio)
                            }
                            _ => None,
                        })
                else {
                    skipped += 1;
                    continue;
                };
                let Some(meta) = parse_dump_caption(message.text()) else {
                    skipped += 1;
                    continue;
                };

                let (file_id, file_unique_id) = file_ids(&document);
                state
                    .rip_deps
                    .tracks()
                    .save_track(&SaveTrackInput {
                        track_key: meta.track_key.clone(),
                        message_id: i64::from(message.id()),
                        file_id,
                        file_unique_id,
                        title: if meta.title.is_empty() {
                            audio.title.as_deref().unwrap_or("Unknown Title").to_owned()
                        } else {
                            meta.title
                        },
                        artist: if meta.artist.is_empty() {
                            audio
                                .performer
                                .as_deref()
                                .unwrap_or("Unknown Artist")
                                .to_owned()
                        } else {
                            meta.artist
                        },
                        album: if meta.album.is_empty() {
                            "Unknown Album".to_owned()
                        } else {
                            meta.album
                        },
                        duration: if meta.duration != 0 {
                            meta.duration
                        } else {
                            i64::from(audio.duration)
                        },
                        bit_depth: meta.bit_depth,
                        sample_rate: meta.sample_rate,
                        genre: if meta.genre.is_empty() {
                            "Music".to_owned()
                        } else {
                            meta.genre
                        },
                        release_date: meta.release_date,
                        track_number: if meta.track_number != 0 {
                            meta.track_number
                        } else {
                            1
                        },
                        track_count: if meta.track_count != 0 {
                            meta.track_count
                        } else {
                            1
                        },
                    })
                    .await
                    .map_err(|error| error.to_string())?;

                valid_track_ids.insert(meta.track_key);
                synced += 1;
            }
        }
        on_progress(scanned, synced);
        let start = max(1, end - 99);
        if start == 1 {
            break;
        }
        end -= 100;
    }

    let valid_ids_vec: Vec<TrackKey> = valid_track_ids.into_iter().collect();
    let pruned = state
        .rip_deps
        .tracks()
        .delete_tracks_not_in(&valid_ids_vec)
        .await
        .map_err(|error| error.to_string())?;

    Ok(IndexSummary {
        scanned,
        synced,
        pruned,
        skipped,
        duration_ms: started.elapsed().as_millis(),
    })
}

fn file_ids(document: &Document) -> (String, String) {
    (
        format!(
            "mtproto:v1:{}:{}:{}",
            document.raw.dc_id,
            document.id(),
            document.access_hash()
        ),
        format!("mtproto:document:{}", document.id()),
    )
}

fn batch_ids(end: i32, batch_size: i32) -> Vec<i32> {
    let start = max(1, end - batch_size + 1);
    (start..=end).rev().collect()
}

fn format_index_summary(summary: &IndexSummary) -> String {
    let time_sec = summary.duration_ms as f64 / 1000.0;
    [
        "✅ <b>Dump Channel Sync Complete</b>".to_owned(),
        String::new(),
        format!(
            "• <b>Messages Scanned:</b> <code>{}</code>",
            summary.scanned
        ),
        format!("• <b>Tracks Synced:</b> <code>{}</code>", summary.synced),
        format!(
            "• <b>Ghost Tracks Pruned:</b> <code>{}</code>",
            summary.pruned
        ),
        format!(
            "• <b>Skipped (non-tracks):</b> <code>{}</code>",
            summary.skipped
        ),
        format!("• <b>Time Elapsed:</b> <code>{time_sec:.1}s</code>"),
    ]
    .join("<br/>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_index_summary_matches_oracle() {
        let summary = IndexSummary {
            scanned: 250,
            synced: 240,
            pruned: 3,
            skipped: 10,
            duration_ms: 4520,
        };
        assert_eq!(
            format_index_summary(&summary),
            "✅ <b>Dump Channel Sync Complete</b><br/><br/>• <b>Messages Scanned:</b> <code>250</code><br/>• <b>Tracks Synced:</b> <code>240</code><br/>• <b>Ghost Tracks Pruned:</b> <code>3</code><br/>• <b>Skipped (non-tracks):</b> <code>10</code><br/>• <b>Time Elapsed:</b> <code>4.5s</code>"
        );
    }

    #[test]
    fn batch_ids_are_descending_and_bounded() {
        let ids = batch_ids(250, 100);
        assert_eq!(ids.len(), 100);
        assert_eq!(ids, (151..=250).rev().collect::<Vec<_>>());
    }
}
