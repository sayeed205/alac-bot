use std::{path::Path, sync::Arc};

use ferogram::{filters, filters::Dispatcher, InputMessage};

use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

const RESTRICTED: &str =
    "🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.";

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let index = ((bytes as f64).ln() / 1024_f64.ln()).floor() as usize;
    let index = index.min(units.len() - 1);
    let value = bytes as f64 / 1024_f64.powi(index as i32);
    if index == 0 {
        format!("{value:.0} {}", units[index])
    } else {
        format!("{value:.2} {}", units[index])
    }
}

async fn clean(msg: ferogram::update::IncomingMessage, state: Arc<BotState>) {
    let sender = msg.sender_user_id().unwrap_or_default();
    if !state.auth.is_admin(sender) {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(RESTRICTED)))
            .await;
        return;
    }

    let downloads_dir = Path::new("bot-data/downloads");
    if !downloads_dir.exists() {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(
                "✨ <b>Clean:</b> Downloads directory is empty.",
            )))
            .await;
        return;
    }

    let result = (|| -> std::io::Result<(usize, u64)> {
        let mut files_removed = 0;
        let mut bytes_freed = 0;
        for entry in std::fs::read_dir(downloads_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".gitkeep" || name == ".gitignore" {
                continue;
            }
            let path = entry.path();
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.is_file() && std::fs::remove_file(&path).is_ok() {
                    bytes_freed += metadata.len();
                    files_removed += 1;
                }
            }
        }
        Ok((files_removed, bytes_freed))
    })();

    match result {
        Ok((files_removed, bytes_freed)) => {
            let text = format!(
                "✓ <b>Temporary storage cleaned</b><br/><br/><blockquote>• Files removed: <code>{files_removed}</code><br/>• Space reclaimed: <code>{}</code><br/>• Target: <code>bot-data/downloads/</code></blockquote>",
                format_bytes(bytes_freed)
            );
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(&text)))
                .await;
        }
        Err(error) => {
            let text = format!(
                "! <b>Could not clean temporary storage.</b><br/><code>{}</code>",
                escape(&error.to_string())
            );
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(&text)))
                .await;
        }
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("clean"), move |msg| {
        clean(msg, Arc::clone(&state))
    });
}
