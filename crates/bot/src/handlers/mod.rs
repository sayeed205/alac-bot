mod auth;
mod help;
mod list;
mod revoke;
#[allow(dead_code)]
pub(crate) mod rip;
pub mod settings;
mod start;
mod status;

#[path = "../callbacks.rs"]
mod callbacks;

use std::sync::Arc;

use ferogram::{
    filters::{self, Dispatcher},
    update::CallbackQuery,
};

use crate::BotState;

/// Bot-API marked group id (-100...) -> t.me/c/ link segment, matching the
/// TS oracle's string slice(4) semantics.
pub(crate) fn group_link_segment(marked_id: i64) -> String {
    let s = marked_id.to_string();
    s.strip_prefix("-100")
        .unwrap_or_else(|| s.trim_start_matches('-'))
        .to_owned()
}

/// Bot-API "marked" peer id for storage and lookups: users stay positive,
/// basic groups are -chat_id, channels/supergroups are -(1e12 + channel_id).
/// Matches what the TS oracle (mtcute) reports as chat.id, which is what the
/// users table stores.
pub(crate) fn marked_peer_id(peer: &ferogram::tl::enums::Peer) -> i64 {
    use ferogram::tl::enums::Peer;
    match peer {
        Peer::User(p) => p.user_id,
        Peer::Chat(p) => -p.chat_id,
        Peer::Channel(p) => -(1_000_000_000_000i64 + p.channel_id),
    }
}

/// Marked chat id for a message's chat, in the storage/DB format.
/// `msg.chat_id()` returns RAW TL ids (positive for channels), which is a
/// different format — never mix the two.
pub(crate) fn marked_chat_id(msg: &ferogram::update::IncomingMessage) -> i64 {
    msg.peer_id()
        .map(marked_peer_id)
        // Peerless updates have no chat; 0 preserves prior behavior.
        .unwrap_or_default()
}

/// Chat peer for sends/edits, derived from the message's TL peer so channel
/// and group ids resolve correctly (PeerRef::from(i64) expects marked ids).
pub(crate) fn chat_peer_ref(msg: &ferogram::update::IncomingMessage) -> ferogram::PeerRef {
    msg.peer_id()
        .map(|peer| ferogram::PeerRef::Peer(peer.clone()))
        // Fallback mirrors prior behavior for peerless updates.
        .unwrap_or(ferogram::PeerRef::from(msg.chat_id()))
}

/// Peer link parity with the TS oracle's getPeerLink: @name -> t.me link,
/// user id -> tg://user, group/channel id -> t.me/c/ link.
pub(crate) fn peer_link(name: &str, id: i64) -> String {
    if let Some(username) = name.strip_prefix('@') {
        format!("https://t.me/{username}")
    } else if id > 0 {
        format!("tg://user?id={id}")
    } else {
        format!("https://t.me/c/{}/1", group_link_segment(id))
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    start::register(dp, Arc::clone(&state));
    help::register(dp, Arc::clone(&state));
    auth::register(dp, Arc::clone(&state));
    revoke::register(dp, Arc::clone(&state));
    list::register(dp, Arc::clone(&state));
    rip::register(dp, Arc::clone(&state));
    status::register(dp, Arc::clone(&state));
    settings::register(dp, Arc::clone(&state));

    let callback_state = Arc::clone(&state);
    dp.on_callback_query(filters::all::<CallbackQuery>(), move |query| {
        let state = Arc::clone(&callback_state);
        async move {
            if query.data().is_some_and(|data| data.starts_with("cancel:")) {
                callbacks::dispatch_cancel(state, query).await;
            } else if query
                .data()
                .is_some_and(|data| data.starts_with("dashboard:"))
            {
                callbacks::dispatch_dashboard(state, query).await;
            } else if query
                .data()
                .is_some_and(|data| data.starts_with("settings:"))
            {
                settings::callback(state, query).await;
            } else {
                list::callback(state, query).await;
            }
        }
    });
}
