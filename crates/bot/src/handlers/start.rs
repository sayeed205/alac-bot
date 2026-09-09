use std::sync::Arc;

use ferogram::{filters, filters::Dispatcher, InputMessage};

use crate::{html::parse_dynamic_html, BotState};

const WELCOME: &str = "<b>Welcome to ALAC Bot</b><br/><br/>\
Send <code>/alac &lt;link&gt;</code> to download lossless audio.<br/><br/>\
Use <code>/help</code> for commands. Audio requested in a group is delivered to your private chat.";

const UNAUTHORIZED: &str =
    "! <b>Welcome to ALAC Bot</b><br/><br/>\
This bot is invite-only. Send your Telegram user ID to the bot owner to request access, then use <code>/start</code> again.<br/><br/>\
After approval, <code>/help</code> shows the full command guide.";

async fn start(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    let chat = msg.peer_id().map(super::marked_peer_id);
    let authorized = state.auth.is_admin(sender)
        || state
            .auth
            .is_authorized(sender, chat)
            .await
            .unwrap_or(false);
    let text = if authorized { WELCOME } else { UNAUTHORIZED };
    if let Err(error) = msg
        .reply(InputMessage::html(parse_dynamic_html(text)))
        .await
    {
        tracing::warn!(%error, "start reply failed");
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("start"), move |msg| {
        start(Arc::clone(&state), msg)
    });
}
