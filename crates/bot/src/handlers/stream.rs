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
    if !msg.is_private() {
        tracing::info!(chat = ?msg.peer_id(), "stream command rejected in non-private chat");
        let text = "⚠️ <b>Direct Message Only.</b><br/>The <code>/stream</code> command can only be used in a private chat with the bot to protect your login credentials.";
        if let Err(err) = msg
            .reply(InputMessage::html(parse_dynamic_html(text)))
            .await
        {
            tracing::error!(error = %err, "failed to reply to group /stream command");
        }
        return;
    }

    let sender = match msg.sender_user_id() {
        Some(id) => id,
        None => {
            tracing::warn!("stream command received without sender_user_id");
            return;
        }
    };

    let is_auth = state.auth.is_admin(sender)
        || state
            .auth
            .is_authorized(sender, None)
            .await
            .unwrap_or(false);

    if !is_auth {
        tracing::warn!(user_id = sender, "unauthorized access attempt to /stream");
        let text = "⚠️ <b>Unauthorized.</b> You are not authorized to access the streaming server.<br/>Contact the bot administrator.";
        if let Err(err) = msg
            .reply(InputMessage::html(parse_dynamic_html(text)))
            .await
        {
            tracing::error!(error = %err, user_id = sender, "failed to reply to unauthorized /stream");
        }
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

    let explicit_track_id: Option<i32> = msg
        .text()
        .and_then(|t| t.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok());

    let sample_track: Option<db::Track> = if let Some(tid) = explicit_track_id {
        state.tracks_repo.find_track_by_id(tid).await.ok().flatten()
    } else {
        state.tracks_repo.find_latest_track().await.ok().flatten()
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

    let (stream_url, direct_section) = if let Some(track) = sample_track {
        let ticket =
            server::streaming::create_playback_ticket(&state.app_key, track.id, sender, 86400);
        let s_url = format!("{clean_url}/api/v1/stream?ticket={ticket}");
        let d_url = format!("{clean_url}/api/v1/stream?track_id={}", track.id);
        let section = format!(
            "▶️ <b>Direct Playback Stream (Track #{}):</b><br/>\
             <b>Title:</b> {} — {}<br/>\
             <b>Format:</b> {} | {}Hz<br/>\
             <b>Direct Stream URL:</b><br/>\
             <code>{}</code><br/><br/>\
             <b>Dev Quick URL:</b><br/>\
             <code>{}</code><br/><br/>",
            track.id,
            crate::html::escape(&track.title),
            crate::html::escape(&track.artist),
            track.codec.as_str().to_uppercase(),
            track.sample_rate,
            s_url,
            d_url,
        );
        (Some(s_url), section)
    } else {
        (None, String::new())
    };

    let text = format!(
        "🎧 <b>Lossless Audio Streaming</b><br/><br/>\
         {direct_section}\
         🔑 <b>Single-Use Login Code:</b><br/>\
         <code>{code}</code><br/><br/>\
         • <b>HTTP Server:</b> <code>{clean_url}</code><br/>\
         • <b>Native App Quick-Connect:</b><br/>\
         <code>{deep_link}</code>"
    );

    let mut input = InputMessage::html(parse_dynamic_html(&text));
    let mut buttons = Vec::new();
    if let Some(ref s_url) = stream_url {
        buttons.push(Button::url("▶️ Stream Now in Browser", s_url));
    }
    if clean_url.starts_with("http://") || clean_url.starts_with("https://") {
        let docs_url = format!("{clean_url}/api/v1/docs");
        buttons.push(Button::url("🌐 Interactive API Docs", docs_url));
    }
    if !buttons.is_empty() {
        let mut kb = InlineKeyboard::new();
        for btn in buttons {
            kb = kb.row([btn]);
        }
        input = input.reply_markup(kb.into_markup());
    }

    if let Err(err) = msg.reply(input).await {
        tracing::error!(error = %err, user_id = sender, "failed to send /stream response");
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("stream"), move |msg| {
        let state = Arc::clone(&state);
        async move {
            stream_command(state, msg).await;
        }
    });
}
