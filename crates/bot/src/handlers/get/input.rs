//! Input handling for the `/get` and `/zip` commands.
//!
//! The parser itself lives in `engine`; this module only deals with Telegram
//! replies and text documents .

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use engine::{
    limits::MAX_DOCUMENT_BYTES,
    parser::{extract_batch_items, parse_alac_input},
    types::ParsedTargetItem,
};
use ferogram::update::IncomingMessage;

#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub items: Vec<ParsedTargetItem>,
    pub force: bool,
    pub storefront: Option<String>,
    pub document: bool,
    pub zip: bool,
}

pub fn command_name(text: &str) -> Option<String> {
    text.split_whitespace().next().map(|s| {
        s.trim_start_matches('/')
            .split('@')
            .next()
            .unwrap_or(s)
            .to_ascii_lowercase()
    })
}

pub fn has_force_token(text: &str) -> bool {
    text.split_whitespace().any(|t| t == "-f" || t == "--force")
}

pub fn has_zip_token(text: &str) -> bool {
    text.split_whitespace().any(|t| t == "-z" || t == "--zip")
}

pub fn parse_text(text: &str, reply: Option<&str>, force_override: bool) -> Option<ParsedCommand> {
    let parsed = parse_alac_input(text, reply)?;
    Some(ParsedCommand {
        items: parsed.items,
        force: force_override || parsed.force,
        zip: parsed.zip,
        storefront: parsed.storefront,
        document: false,
    })
}

fn temp_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("batch_{stamp}.txt"))
}

/// Parse a command, resolving a replied-to message and an attached `.txt`
/// document. Failed document downloads deliberately fall back to normal text,
/// just as the live bot does.
///
/// `chat_id` is the Bot-API marked chat id (negative for channels/supergroups).
/// It is used to prime the peer cache before fetching the reply message, so
/// `channels.getMessages` works even immediately after a fresh bot start.
pub async fn parse_message(
    client: &ferogram::Client,
    message: &IncomingMessage,
    chat_id: i64,
    force_override: bool,
) -> ParsedCommand {
    // Prime the peer cache for this chat so channels.getMessages has a valid
    // access_hash. On a cache hit this is a no-op (just a local map read);
    // on a cache miss (fresh start) it does one cheap RPC to fetch the chat.
    if message.reply_to_message_id().is_some() {
        if let Err(e) = client.resolve(ferogram::PeerRef::Id(chat_id)).await {
            tracing::warn!(chat_id, error = %e, "get: could not prime peer cache for chat");
        }
    }

    let reply = message.get_reply_with(client).await.ok().flatten();
    let document = message
        .document()
        .or_else(|| reply.as_ref().and_then(IncomingMessage::document));
    if let Some(document) = document {
        let name = document.file_name().unwrap_or("").to_ascii_lowercase();
        let mime = document.mime_type().to_ascii_lowercase();
        if name.ends_with(".txt") || mime == "text/plain" || mime.contains("text/plain") {
            let path = temp_path();
            let result: Result<String, String> = async {
                client
                    .download_file(&document, &path)
                    .await
                    .map_err(|e| e.to_string())?;
                let size = tokio::fs::metadata(&path)
                    .await
                    .map_err(|e| e.to_string())?
                    .len();
                if size > MAX_DOCUMENT_BYTES {
                    return Err(format!(
                        "text document exceeds the {} MiB limit",
                        MAX_DOCUMENT_BYTES / (1024 * 1024)
                    ));
                }
                let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
                let content = if let Ok(s) = std::str::from_utf8(&bytes) {
                    s.trim_start_matches('\u{FEFF}').to_owned()
                } else {
                    String::from_utf8_lossy(&bytes)
                        .trim_start_matches('\u{FEFF}')
                        .to_owned()
                };
                Ok(content)
            }
            .await;
            let _ = tokio::fs::remove_file(&path).await;
            if let Ok(content) = result {
                return ParsedCommand {
                    items: extract_batch_items(&content),
                    force: force_override || message.text().is_some_and(has_force_token),
                    zip: message.text().is_some_and(has_zip_token),
                    storefront: None,
                    document: true,
                };
            }
        }
    }
    parse_text(
        message.text().unwrap_or_default(),
        reply.as_ref().and_then(IncomingMessage::text),
        force_override,
    )
    .unwrap_or(ParsedCommand {
        items: Vec::new(),
        force: force_override,
        storefront: None,
        document: false,
        zip: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_and_flags_are_case_insensitive() {
        assert_eq!(command_name("/GET@bot 1"), Some("get".into()));
        assert!(has_force_token("/get --force 1"));
        assert!(has_zip_token("/get --zip 1"));
    }
}
