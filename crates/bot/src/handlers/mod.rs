mod auth;
mod help;
mod list;
mod revoke;
mod start;

use std::sync::Arc;

use ferogram::filters::{self, Dispatcher};
use ferogram::update::CallbackQuery;

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

    let callback_state = Arc::clone(&state);
    dp.on_callback_query(filters::all::<CallbackQuery>(), move |query| {
        let state = Arc::clone(&callback_state);
        async move { list::callback(state, query).await }
    });
}
