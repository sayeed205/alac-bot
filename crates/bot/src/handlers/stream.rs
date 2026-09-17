//! `/stream` command — generates single-use OTP and native app connection links.

use std::sync::Arc;

use ferogram::{
    filters,
    filters::Dispatcher,
    keyboard::{Button, InlineKeyboard},
    InputMessage,
};

use crate::{html::parse_dynamic_html, BotState};

pub async fn stream_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = match msg.sender_user_id() {
        Some(id) => id,
        None => return,
    };

    let is_auth = state.auth.is_admin(sender)
        || state
            .auth
            .is_authorized(sender, None)
            .await
            .unwrap_or(false);

    if !is_auth {
        let text = "⚠️ <b>Unauthorized.</b> You are not authorized to access the streaming server.<br/>Contact the bot administrator.";
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(text)))
            .await;
        return;
    }

    let code = match state.session_manager.create_login_code(sender).await {
        Ok(code) => code,
        Err(err) => {
            tracing::error!(error = %err, user_id = sender, "failed to generate stream login code");
            let text = "⚠️ <b>Error:</b> Failed to generate login code. Please try again later.";
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(text)))
                .await;
            return;
        }
    };

    let settings = state.rip_deps.settings_snapshot();
    let default_url = format!("http://127.0.0.1:{}", settings.stream_server_port);
    let public_url = settings
        .stream_public_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or(&default_url);

    let clean_url = public_url.trim_end_matches('/');
    let deep_link = format!("alac://auth?code={code}&server={clean_url}");
    let text = format!(
        "🎧 <b>Lossless Audio Streaming Login</b><br/><br/>\
         Your one-time login code is:<br/>\
         <code>{code}</code><br/><br/>\
         • <b>Valid for:</b> 5 minutes<br/>\
         • <b>Server:</b> <code>{clean_url}</code><br/><br/>\
         Tap the button below or enter the code in your native player app to connect."
    );
    let kb = InlineKeyboard::new()
        .row([Button::url("🎧 Connect in App", deep_link)])
        .into_markup();

    let input = InputMessage::html(parse_dynamic_html(&text)).reply_markup(kb);

    let _ = msg.reply(input).await;
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("stream"), move |msg| {
        let state = Arc::clone(&state);
        async move {
            stream_command(state, msg).await;
        }
    });
}
