//! `/export` + `/import` — gzipped typed database archive backup/restore
//! (oracle: `src/modules/alac/commands/backup.ts`, service
//! `src/db/dump.ts` → `db::DbDumpService`).
//!
//! Both commands are admin-only AND DM-only (the oracle silently returns
//! outside a private chat).

use std::sync::Arc;

use ferogram::{filters, filters::Dispatcher, InputMessage, PeerRef};

use crate::{
    handlers::chat_peer_ref,
    html::{escape, parse_dynamic_html},
    BotState,
};

const KB: f64 = 1024.0;

/// Oracle caption: "Compressed Size" in KB with one decimal.
fn export_caption(users: i64, tracks: i64, requests: i64, bytes: usize) -> String {
    format!(
        "<b>📦 Database Dump Exported</b>\n\n\
• <b>Users:</b> {users}\n\
• <b>Tracks:</b> {tracks}\n\
• <b>Requests:</b> {requests}\n\
• <b>Compressed Size:</b> {:.1} KB",
        bytes as f64 / KB
    )
}

/// Extract the `.json.gz` (or `.gz`) document from the replied-to message.
async fn replied_document(
    state: &BotState,
    msg: &ferogram::update::IncomingMessage,
) -> Option<(ferogram::tl::enums::MessageMedia, String)> {
    let reply_id = msg.reply_to_message_id()?;
    let peer = chat_peer_ref(msg);
    let messages = state.client.get_messages(peer, &[reply_id]).await.ok()?;
    let reply = messages.first()?;
    let media = reply.media()?.clone();
    let doc = ferogram::media::Document::from_media(&media)?;
    let name = doc.file_name().unwrap_or_default().to_owned();
    Some((media, name))
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    let export_state = Arc::clone(&state);
    dp.on_message(filters::command("export"), move |msg| {
        let state = Arc::clone(&export_state);
        async move {
            export(state, msg).await;
        }
    });

    let import_state = Arc::clone(&state);
    dp.on_message(filters::command("import"), move |msg| {
        let state = Arc::clone(&import_state);
        async move {
            import(state, msg).await;
        }
    });
}

async fn export(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    if !state
        .auth
        .is_admin(msg.sender_user_id().unwrap_or_default())
    {
        return;
    }
    if !msg.is_private() {
        return;
    }
    let Some(peer) = msg.peer_id() else { return };
    let peer = PeerRef::Peer(peer.clone());

    let status = msg
        .reply(InputMessage::html(parse_dynamic_html(
            "📦 <i>Generating database export...</i>",
        )))
        .await
        .ok();

    let dump = db::DbDumpService::new(state.db_client.clone());
    let result = dump.export_dump().await;
    let (buffer, stats, _filename) = match result {
        Ok(ok) => ok,
        Err(error) => {
            let text = format!(
                "❌ <b>Export Failed</b>\n\n<code>{}</code>",
                escape(&error.to_string())
            );
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(&text)))
                .await;
            if let Some(status) = status {
                let _ = status.delete().await;
            }
            return;
        }
    };

    // Write to a temp file, upload as a document, then clean up.
    let tmp = std::env::temp_dir().join(format!(
        "alac_export_{}.json.gz",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    ));
    if let Err(error) = std::fs::write(&tmp, &buffer) {
        let text = format!(
            "❌ <b>Export Failed</b>\n\n<code>{}</code>",
            escape(&error.to_string())
        );
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(&text)))
            .await;
        return;
    }

    let caption = export_caption(
        stats.users_count,
        stats.tracks_count,
        stats.requests_count,
        stats.bytes,
    );
    let send_result = match state.client.upload_file(&tmp).await {
        Ok(uploaded) => {
            let media = uploaded.as_document_media();
            let input = InputMessage::html(parse_dynamic_html(&caption)).copy_media(media);
            state
                .client
                .send_message(peer.clone(), input)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    };

    let _ = std::fs::remove_file(&tmp);

    if let Err(error) = send_result {
        let text = format!("❌ <b>Export Failed</b>\n\n<code>{}</code>", escape(&error));
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(&text)))
            .await;
    }

    // Oracle deletes the "Generating..." status message in a finally.
    if let Some(status) = status {
        let _ = status.delete().await;
    }
}

async fn import(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    if !state
        .auth
        .is_admin(msg.sender_user_id().unwrap_or_default())
    {
        return;
    }
    if !msg.is_private() {
        return;
    }
    if msg.peer_id().is_none() {
        return;
    }

    let Some((doc, file_name)) = replied_document(&state, &msg).await else {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(
                "⚠️ Please reply to a valid <code>.json.gz</code> database archive with <code>/import</code> to restore.",
            )))
            .await;
        return;
    };
    if !file_name.ends_with(".json.gz") && !file_name.ends_with(".gz") {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(
                "⚠️ The replied file must be a <code>.json.gz</code> database archive.",
            )))
            .await;
        return;
    }

    let status = msg
        .reply(InputMessage::html(parse_dynamic_html(
            "⏳ <i>Downloading and restoring database dump...</i>",
        )))
        .await
        .ok();

    let tmp = std::env::temp_dir().join(format!(
        "alac_import_{}.json.gz",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    ));

    let restore_result = match download_document(&state, &doc, &tmp).await {
        Ok(bytes) => {
            let dump = db::DbDumpService::new(state.db_client.clone());
            dump.import_dump(&bytes).await.map_err(|e| e.to_string())
        }
        Err(error) => Err(error),
    };

    let _ = std::fs::remove_file(&tmp);

    let text = match restore_result {
        Ok(stats) => format!(
            "<b>✅ Database Restored Successfully!</b>\n\n\
• <b>Users Merged:</b> {}\n\
• <b>Tracks Merged:</b> {}\n\
• <b>Requests Merged:</b> {}\n\
• <b>Elapsed Time:</b> {}ms",
            stats.users_merged, stats.tracks_merged, stats.requests_merged, stats.duration_ms
        ),
        Err(error) => format!(
            "<b>❌ Database Restore Failed</b>\n\n<code>{}</code>\n\n<i>Transaction rolled back. Database state remains unchanged.</i>",
            escape(&error)
        ),
    };

    let _ = msg
        .reply(InputMessage::html(parse_dynamic_html(&text)))
        .await;

    // Oracle deletes the status message in a finally.
    if let Some(status) = status {
        let _ = status.delete().await;
    }
}

/// Download a document's bytes to a temp path and read them back.
async fn download_document(
    state: &BotState,
    media: &ferogram::tl::enums::MessageMedia,
    tmp: &std::path::Path,
) -> Result<Vec<u8>, String> {
    use ferogram::media::MediaQuality;
    state
        .client
        .download_media(media, MediaQuality::Original, tmp, None)
        .await
        .map_err(|error| error.to_string())?;
    std::fs::read(tmp).map_err(|error| error.to_string())
}
