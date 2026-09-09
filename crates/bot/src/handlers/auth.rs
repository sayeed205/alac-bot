use std::sync::Arc;

use ferogram::{filters, filters::Dispatcher, tl, InputMessage, PeerRef};

use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

/// Resolve a TL peer to the Bot-API "marked" id the TS oracle stores
/// (users positive, basic groups -chat_id, channels -1e12 - channel_id).
fn peer_id(peer: &tl::enums::Peer) -> (i64, bool) {
    match peer {
        tl::enums::Peer::User(p) => (p.user_id, true),
        tl::enums::Peer::Chat(p) => (-p.chat_id, false),
        tl::enums::Peer::Channel(p) => (-(1_000_000_000_000i64 + p.channel_id), false),
    }
}

pub(crate) async fn resolve_target(
    msg: &ferogram::update::IncomingMessage,
    state: &BotState,
) -> Option<(i64, String, bool)> {
    // 1. Reply target: fetch the replied-to message and use its sender.
    if let Some(reply_id) = msg.reply_to_message_id() {
        if let Some(peer) = msg.peer_id() {
            if let Ok(messages) = state
                .client
                .get_messages(PeerRef::Peer(peer.clone()), &[reply_id])
                .await
            {
                if let Some(reply) = messages.first() {
                    if let Some(peer) = reply.sender_id() {
                        let (id, user) = peer_id(peer);
                        // Display-name parity: displayName || username ||
                        // "User {id}". sender_user() does a users.getUsers
                        // fetch; fall back to the id form on error.
                        let name = match reply.sender_user().await {
                            Ok(Some(u)) => u
                                .first_name()
                                .map(str::to_owned)
                                .map(|first| match u.last_name() {
                                    Some(last) => format!("{first} {last}"),
                                    None => first,
                                })
                                .or_else(|| u.username().map(str::to_owned))
                                .unwrap_or_else(|| format!("User {id}")),
                            _ => format!("User {id}"),
                        };
                        return Some((id, name, user));
                    }
                }
            }
        }
    }
    // 2. Explicit argument: numeric id or @username.
    let text = msg.text()?.split_whitespace().nth(1)?;
    if let Ok(id) = text.parse::<i64>() {
        return Some((id, format!("User {id}"), id > 0));
    }
    if text.starts_with('@') {
        if let Ok(peer) = state
            .client
            .resolve(PeerRef::Username(text.to_owned()))
            .await
        {
            let (id, user) = peer_id(&peer);
            return Some((id, text.to_owned(), user));
        }
    }
    // 3. No argument inside a group: authorize the group chat itself, using
    //    the marked id form the TS oracle stores.
    if msg.is_any_group() {
        let marked = match msg.peer_id() {
            Some(tl::enums::Peer::Chat(c)) => -c.chat_id,
            Some(tl::enums::Peer::Channel(c)) => -(1_000_000_000_000i64 + c.channel_id),
            _ => return None,
        };
        return Some((marked, format!("Chat {marked}"), false));
    }
    None
}

pub(crate) async fn authorize(msg: ferogram::update::IncomingMessage, state: Arc<BotState>) {
    let sender = match msg.sender_user_id() {
        Some(id) => id,
        None => return,
    };
    if !state.auth.is_admin(sender) {
        return;
    }
    let Some((id, name, is_user)) = resolve_target(&msg, &state).await else {
        let text = "<b>Authorization usage</b><br/><br/><blockquote>• Reply to a message with <code>/auth</code><br/>• <code>/auth &lt;id | @username&gt;</code><br/>• Send <code>/auth</code> inside a group to authorize the whole group</blockquote>";
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(text)))
            .await;
        return;
    };
    let link = super::peer_link(&name, id);
    match state.auth.authorize(id, Some(&name)).await {
        Ok(new) => {
            let word = if new { "Authorized" } else { "Updated" };
            let kind = if is_user { "User" } else { "Group" };
            let text = format!(
                "✓ <b>{word} {kind}:</b> {} <a href=\"{link}\">{}</a> (<code>{id}</code>)",
                if is_user { "👤" } else { "👥" },
                escape(&name)
            );
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(&text)))
                .await;
        }
        Err(error) => tracing::error!(%error, "authorization failed"),
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    let auth_state = Arc::clone(&state);
    dp.on_message(filters::command("auth"), move |msg| {
        authorize(msg, Arc::clone(&auth_state))
    });
}
