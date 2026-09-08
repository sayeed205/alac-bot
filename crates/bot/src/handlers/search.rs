//! `/search` — local lossless cache + live Apple Music catalog search
//! (oracle: `src/modules/alac/commands/search.ts`), plus the `dl:`/`rip:`
//! delivery callbacks and `search_close`.

use std::sync::Arc;

use engine::{
    orchestrator::deps::OrchestratorDeps,
    types::{ParsedTargetItem, TargetKind, TrackMeta},
};
use ferogram::{
    filters::{self, Dispatcher},
    keyboard::{Button, InlineKeyboard},
    update::{CallbackQuery, IncomingMessage},
    InputMessage, PeerRef,
};

use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

/// Oracle truncation: `{artist} - {title}` over 28 chars → first 25 + "...".
fn short_title(artist: &str, title: &str) -> String {
    let raw = format!("{artist} - {title}");
    if raw.chars().count() > 28 {
        let truncated: String = raw.chars().take(25).collect();
        format!("{truncated}...")
    } else {
        raw
    }
}

/// Oracle quality suffix: ` [bit/…kHz]` only when both fields are set
/// (zero is the DB "absent" sentinel).
fn cached_quality(bit_depth: i32, sample_rate: i32) -> String {
    if bit_depth == 0 || sample_rate == 0 {
        return String::new();
    }
    let khz = (sample_rate as f64) / 1000.0;
    format!(" [{}-bit/{khz:.1}kHz]", bit_depth)
}

/// (title, artist) pairs for rendering; cached rows carry a quality suffix.
type Row = (String, String, String);

/// Builds the exact search-results text (oracle lines 77-135).
fn build_results_html(query: &str, cached: &[Row], live: &[Row]) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !cached.is_empty() {
        let lines = cached
            .iter()
            .enumerate()
            .map(|(index, (title, artist, quality))| {
                format!(
                    "{}. <b>{}</b> — <i>{}</i><code>{}</code>",
                    index + 1,
                    escape(title),
                    escape(artist),
                    quality
                )
            })
            .collect::<Vec<_>>()
            .join("<br/>");
        sections.push(format!(
            "⚡ <b>Instant Lossless Cache:</b><br/><blockquote>{lines}</blockquote>"
        ));
    }
    if !live.is_empty() {
        let start = cached.len() + 1;
        let lines = live
            .iter()
            .enumerate()
            .map(|(index, (title, artist, _))| {
                format!(
                    "{}. <b>{}</b> — <i>{}</i>",
                    start + index,
                    escape(title),
                    escape(artist)
                )
            })
            .collect::<Vec<_>>()
            .join("<br/>");
        sections.push(format!(
            "🎵 <b>Apple Music Catalog:</b><br/><blockquote>{lines}</blockquote>"
        ));
    }
    format!(
        "🔍 <b>Search results for \"<i>{}</i>\":</b><br/><br/>{}<br/><br/><i>Tap ⚡ for instant cache delivery or 🎵 to rip ALAC lossless:</i>",
        escape(query),
        sections.join("<br/><br/>")
    )
}

/// Keyboard rows: cached `dl:{id}` rows then live `rip:{id}` rows, Close last.
fn build_results_keyboard(
    cached: &[(String, String, String)],
    live: &[(String, String, String)],
) -> ferogram::tl::enums::ReplyMarkup {
    let mut kb = InlineKeyboard::new();
    for (index, (id, title, artist)) in cached.iter().enumerate() {
        kb = kb.row(vec![Button::callback(
            format!("⚡ {}. {}", index + 1, short_title(artist, title)),
            format!("dl:{id}").as_bytes(),
        )]);
    }
    let start = cached.len() + 1;
    for (index, (id, title, artist)) in live.iter().enumerate() {
        kb = kb.row(vec![Button::callback(
            format!("🎵 {}. {}", start + index, short_title(artist, title)),
            format!("rip:{id}").as_bytes(),
        )]);
    }
    kb = kb.row(vec![Button::callback("❌ Close", b"search_close")]);
    kb.into_markup()
}

/// Gate text shared by the command surface.
const PAUSED: &str =
    "⚠️ <b>Service is temporarily paused for maintenance.</b><br/>Please check back later.";

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    let search_state = Arc::clone(&state);
    dp.on_message(filters::command("search"), move |msg| {
        search(Arc::clone(&search_state), msg)
    });
}

async fn search(state: Arc<BotState>, msg: IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    let marked_chat = super::marked_chat_id(&msg);
    if !state
        .auth
        .is_authorized(sender, Some(marked_chat))
        .await
        .unwrap_or(false)
    {
        return;
    }
    let is_admin = state.auth.is_admin(sender);
    if !state.rip_deps.settings_snapshot().can_serve_cache(is_admin) {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(PAUSED)))
            .await;
        return;
    }

    let query = msg
        .text()
        .unwrap_or_default()
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        let usage = "🔍 <b>Search Music:</b><br/><br/><blockquote><b>Usage:</b> <code>/search &lt;track title or artist&gt;</code><br/><i>Searches both local lossless cache and the live Apple Music catalog.</i></blockquote>";
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(usage)))
            .await;
        return;
    }

    let (cached, live) = tokio::join!(
        state.rip_deps.tracks().search_cached_tracks(&query, 5),
        state.rip_deps.catalog().search_catalog(&query, 10, "us"),
    );
    let cached = cached.unwrap_or_default();
    let live = live.unwrap_or_default();

    if cached.is_empty() && live.is_empty() {
        let text = format!(
            "🔍 No tracks found for \"<b>{}</b>\". Try refining your search query!",
            escape(&query)
        );
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(&text)))
            .await;
        return;
    }

    let cached_ids: std::collections::HashSet<&str> =
        cached.iter().map(|t| t.apple_track_id.as_str()).collect();
    let uncached_live: Vec<&TrackMeta> = live
        .iter()
        .filter(|t| !cached_ids.contains(t.id.as_str()))
        .collect();

    let cached_rows: Vec<Row> = cached
        .iter()
        .map(|t| {
            (
                t.title.clone(),
                t.artist.clone(),
                cached_quality(t.bit_depth, t.sample_rate),
            )
        })
        .collect();
    let live_rows: Vec<Row> = uncached_live
        .iter()
        .map(|t| (t.title.clone(), t.artist.clone(), t.id.clone()))
        .collect();

    let text = build_results_html(&query, &cached_rows, &live_rows);
    let keyboard = build_results_keyboard(
        &cached
            .iter()
            .map(|t| (t.apple_track_id.clone(), t.title.clone(), t.artist.clone()))
            .collect::<Vec<_>>(),
        &uncached_live
            .iter()
            .map(|t| (t.id.clone(), t.title.clone(), t.artist.clone()))
            .collect::<Vec<_>>(),
    );

    let _ = msg
        .reply(InputMessage::html(parse_dynamic_html(&text)).reply_markup(keyboard))
        .await;
}

pub async fn callback(state: Arc<BotState>, query: CallbackQuery) {
    let data = query.data().unwrap_or_default().to_owned();
    if data == "search_close" {
        close(state, query).await;
    } else if let Some(track_id) = data.strip_prefix("dl:") {
        deliver_cached(state, query, track_id.to_owned()).await;
    } else if let Some(track_id) = data.strip_prefix("rip:") {
        rip(state, query, track_id.to_owned()).await;
    }
}

async fn close(state: Arc<BotState>, query: CallbackQuery) {
    let _ = query.answer().send(&state.client).await;
    delete_query_message(&state, &query).await;
}

async fn deliver_cached(state: Arc<BotState>, query: CallbackQuery, track_id: String) {
    let marked_chat = query
        .chat_peer
        .as_ref()
        .map(super::marked_peer_id)
        .unwrap_or(query.user_id);
    if !state
        .auth
        .is_authorized(query.user_id, Some(marked_chat))
        .await
        .unwrap_or(false)
    {
        let _ = query
            .answer()
            .alert("Unauthorized")
            .send(&state.client)
            .await;
        return;
    }
    let is_admin = state.auth.is_admin(query.user_id);
    if !state.rip_deps.settings_snapshot().can_serve_cache(is_admin) {
        let _ = query
            .answer()
            .alert("⚠️ Service is temporarily paused for maintenance.")
            .send(&state.client)
            .await;
        return;
    }
    let cached = state
        .rip_deps
        .find_cached_tracks(std::slice::from_ref(&track_id))
        .await
        .ok()
        .and_then(|mut map| map.remove(&track_id));
    let Some(cached) = cached else {
        let _ = query
            .answer()
            .alert("Track is no longer cached in dump channel.")
            .send(&state.client)
            .await;
        return;
    };
    let _ = query
        .answer()
        .text("⚡ Delivering lossless track from cache!")
        .send(&state.client)
        .await;
    if let Err(error) = state
        .rip_deps
        .sink()
        .send_dump_copy(marked_chat, cached.message_id, None, false)
        .await
    {
        tracing::warn!(track_id, %error, "failed to deliver cached track");
        let _ = query
            .answer()
            .alert("Failed to retrieve audio from dump channel.")
            .send(&state.client)
            .await;
        return;
    }
    delete_query_message(&state, &query).await;
    let _ = state
        .rip_deps
        .log_request(engine::orchestrator::deps::RequestLog {
            telegram_id: query.user_id,
            chat_id: marked_chat,
            apple_track_id: track_id,
            is_cache_hit: true,
            duration_ms: Some(100),
            status: "completed".to_owned(),
            error_reason: None,
        })
        .await;
}

async fn rip(state: Arc<BotState>, query: CallbackQuery, track_id: String) {
    let marked_chat = query
        .chat_peer
        .as_ref()
        .map(super::marked_peer_id)
        .unwrap_or(query.user_id);
    if !state
        .auth
        .is_authorized(query.user_id, Some(marked_chat))
        .await
        .unwrap_or(false)
    {
        let _ = query
            .answer()
            .alert("Unauthorized")
            .send(&state.client)
            .await;
        return;
    }
    let is_admin = state.auth.is_admin(query.user_id);

    let cached = state
        .rip_deps
        .find_cached_tracks(std::slice::from_ref(&track_id))
        .await
        .ok()
        .and_then(|mut map| map.remove(&track_id));
    if let Some(cached) = cached {
        if !state.rip_deps.settings_snapshot().can_serve_cache(is_admin) {
            let _ = query
                .answer()
                .alert("⚠️ Service is temporarily paused for maintenance.")
                .send(&state.client)
                .await;
            return;
        }
        let _ = query
            .answer()
            .text("⚡ Already cached! Delivering track...")
            .send(&state.client)
            .await;
        let _ = state
            .rip_deps
            .sink()
            .send_dump_copy(marked_chat, cached.message_id, None, false)
            .await;
        delete_query_message(&state, &query).await;
        return;
    }

    if !state.rip_deps.settings_snapshot().can_rip_live(is_admin) {
        let _ = query
            .answer()
            .alert("⚠️ Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now.")
            .send(&state.client)
            .await;
        return;
    }

    let _ = query
        .answer()
        .text("⏳ Queuing lossless rip...")
        .send(&state.client)
        .await;

    let peer = query
        .chat_peer
        .as_ref()
        .map(|peer| PeerRef::Peer(peer.clone()))
        .unwrap_or_else(|| PeerRef::from(query.user_id));
    let Ok(status) = state
        .client
        .send_message(
            peer,
            InputMessage::html(parse_dynamic_html(&format!(
                "⏳ <b>Queuing track {track_id} for ripping...</b>"
            )))
            .reply_to(query.message_id),
        )
        .await
    else {
        return;
    };

    let options = engine::orchestrator::types::RipJobOptions {
        chat_id: marked_chat,
        user_id: query.user_id,
        user_name: Some(format!("User {}", query.user_id)),
        delivery_chat_id: marked_chat,
        is_group: marked_chat != query.user_id,
        is_force: false,
        is_cache_only: false,
        single_storefront: None,
        parsed_items: vec![ParsedTargetItem {
            id: track_id.clone(),
            kind: TargetKind::Track,
            storefront: None,
        }],
        reply_to_message_id: None,
        status_msg_id: i64::from(status.id()),
        is_admin,
    };
    match state
        .rip_orchestrator
        .start_job(Arc::clone(&state.rip_deps), &options)
        .await
    {
        Ok(_) => {
            delete_query_message(&state, &query).await;
        }
        Err(error) => {
            tracing::warn!(track_id, %error, "search rip job failed");
        }
    }
}

async fn delete_query_message(state: &BotState, query: &CallbackQuery) {
    let Some(message_id) = query.message_id else {
        return;
    };
    let Some(peer) = query.chat_peer.as_ref() else {
        return;
    };
    let peer = PeerRef::Peer(peer.clone());
    if let Ok(messages) = state.client.get_messages(peer, &[message_id]).await {
        if let Some(message) = messages.first() {
            let _ = message.delete().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_title_truncates_like_oracle() {
        assert_eq!(short_title("A", "B"), "A - B");
        // 5 + 3 + 20 = exactly 28 chars: untouched.
        let b28 = "B".repeat(20);
        assert_eq!(short_title("AAAAA", &b28), format!("AAAAA - {b28}"));
        // Any string over 28 chars: first 25 + "..." (computed, not hand-counted).
        let over = format!("AAAAA - {}", "B".repeat(21));
        assert_eq!(
            short_title("AAAAA", &"B".repeat(21)),
            format!("{}...", over.chars().take(25).collect::<String>())
        );
        let far_over = format!("AAAAA - {}", "B".repeat(50));
        assert_eq!(
            short_title("AAAAA", &"B".repeat(50)),
            format!("{}...", far_over.chars().take(25).collect::<String>())
        );
    }

    #[test]
    fn quality_suffix_matches_oracle() {
        assert_eq!(cached_quality(0, 0), "");
        assert_eq!(cached_quality(24, 48000), " [24-bit/48.0kHz]");
        assert_eq!(cached_quality(16, 44100), " [16-bit/44.1kHz]");
    }

    #[test]
    fn results_html_is_exact() {
        let cached = vec![
            (
                "Cached One".to_owned(),
                "Artist A".to_owned(),
                " [24-bit/48.0kHz]".to_owned(),
            ),
            (
                "Cached Two".to_owned(),
                "Artist B".to_owned(),
                String::new(),
            ),
        ];
        let live = vec![(
            "Live One".to_owned(),
            "Artist C".to_owned(),
            "id9".to_owned(),
        )];
        let text = build_results_html("query", &cached, &live);
        assert_eq!(
            text,
            "🔍 <b>Search results for \"<i>query</i>\":</b><br/><br/>⚡ <b>Instant Lossless Cache:</b><br/><blockquote>1. <b>Cached One</b> — <i>Artist A</i><code> [24-bit/48.0kHz]</code><br/>2. <b>Cached Two</b> — <i>Artist B</i><code></code></blockquote><br/><br/>🎵 <b>Apple Music Catalog:</b><br/><blockquote>3. <b>Live One</b> — <i>Artist C</i></blockquote><br/><br/><i>Tap ⚡ for instant cache delivery or 🎵 to rip ALAC lossless:</i>"
        );
    }

    #[test]
    fn results_html_cached_only_and_live_only() {
        let cached = vec![("C".to_owned(), "A".to_owned(), String::new())];
        assert_eq!(
            build_results_html("q", &cached, &[]),
            "🔍 <b>Search results for \"<i>q</i>\":</b><br/><br/>⚡ <b>Instant Lossless Cache:</b><br/><blockquote>1. <b>C</b> — <i>A</i><code></code></blockquote><br/><br/><i>Tap ⚡ for instant cache delivery or 🎵 to rip ALAC lossless:</i>"
        );
        let live = vec![("L".to_owned(), "B".to_owned(), String::new())];
        assert_eq!(
            build_results_html("q", &[], &live),
            "🔍 <b>Search results for \"<i>q</i>\":</b><br/><br/>🎵 <b>Apple Music Catalog:</b><br/><blockquote>1. <b>L</b> — <i>B</i></blockquote><br/><br/><i>Tap ⚡ for instant cache delivery or 🎵 to rip ALAC lossless:</i>"
        );
    }
}
