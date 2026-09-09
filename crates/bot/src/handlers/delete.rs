use std::sync::Arc;

use engine::{parser::parse_alac_input, Provider, TrackKey};
use ferogram::{filters, filters::Dispatcher, InputMessage, PeerRef};

use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

const RESTRICTED: &str =
    "🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.";
const USAGE: &str = "🗑️ <b>Delete Track Usage:</b><br/><br/><blockquote>• <code>/delete &lt;apple_music_link | track_id&gt;</code><br/>• Reply to an Apple Music link with <code>/delete</code></blockquote>";

async fn reply_text(msg: &ferogram::update::IncomingMessage, state: &BotState) -> Option<String> {
    let reply_id = msg.reply_to_message_id()?;
    let peer = msg.peer_id()?.clone();
    let messages = state
        .client
        .get_messages(PeerRef::Peer(peer), &[reply_id])
        .await
        .ok()?;
    messages
        .first()
        .and_then(|message| message.text().map(str::to_owned))
}

async fn delete(msg: ferogram::update::IncomingMessage, state: Arc<BotState>) {
    let sender = msg.sender_user_id().unwrap_or_default();
    if !state.auth.is_admin(sender) {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(RESTRICTED)))
            .await;
        return;
    }

    let reply = reply_text(&msg, &state).await;
    let parsed = parse_alac_input(msg.text().unwrap_or(""), reply.as_deref());
    let Some(parsed) = parsed else {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(USAGE)))
            .await;
        return;
    };
    let track_id = parsed.track_id;
    let track_key = TrackKey::new(Provider::Apple, track_id.clone());

    let cached = match state
        .rip_deps
        .tracks()
        .find_cached_tracks(std::slice::from_ref(&track_key))
        .await
    {
        Ok(mut tracks) => tracks.remove(&track_key),
        Err(error) => {
            tracing::warn!(%error, "failed to find cached track for deletion");
            return;
        }
    };
    let Some(cached) = cached else {
        let text = format!(
            "⚠️ <b>Track Not Found:</b> ID <code>{track_id}</code> is not in the database."
        );
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(&text)))
            .await;
        return;
    };

    if let Ok(message_id) = i32::try_from(cached.message_id) {
        if let Ok(messages) = state
            .client
            .get_messages(state.dump_peer.clone(), &[message_id])
            .await
        {
            if let Some(message) = messages.first() {
                let _ = message.delete().await;
            }
        }
    }

    if let Err(error) = state.rip_deps.tracks().delete_track(&track_key).await {
        tracing::warn!(%error, track_id, "failed to delete cached track");
        return;
    }

    let title = if cached.title.is_empty() {
        format!("Track {track_id}")
    } else {
        cached.title.clone()
    };
    let title = escape(&title);
    let artist = if cached.artist.is_empty() {
        "Unknown Artist".to_owned()
    } else {
        cached.artist.clone()
    };
    let artist = escape(&artist);
    let text = format!(
        "🗑️ <b>Track Deleted Successfully</b><br/><br/><blockquote><b>Details:</b><br/>• Title: <b>{title}</b> — {artist}<br/>• Apple ID: <code>{track_id}</code><br/>• Dump Message: <code>#{}</code><br/>• Purged from database & dump channel.</blockquote>",
        cached.message_id
    );
    let _ = msg
        .reply(InputMessage::html(parse_dynamic_html(&text)))
        .await;
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("delete"), move |msg| {
        delete(msg, Arc::clone(&state))
    });
}
