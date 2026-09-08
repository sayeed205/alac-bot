//! Input handling for the `/alac` family of commands.
//!
//! The parser itself lives in `engine`; this module only deals with Telegram
//! replies and text documents (oracle `commands-rip.ts:125-177`).

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use engine::{
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

pub fn parse_text(text: &str, reply: Option<&str>, rerip: bool) -> Option<ParsedCommand> {
    let parsed = parse_alac_input(text, reply)?;
    Some(ParsedCommand {
        items: parsed.items,
        force: rerip || parsed.force,
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
pub async fn parse_message(
    client: &ferogram::Client,
    message: &IncomingMessage,
    rerip: bool,
) -> ParsedCommand {
    let reply = message.get_reply_with(client).await.ok().flatten();
    let document = message
        .document()
        .or_else(|| reply.as_ref().and_then(IncomingMessage::document));
    if let Some(document) = document {
        let name = document.file_name().unwrap_or("").to_ascii_lowercase();
        let mime = document.mime_type().to_ascii_lowercase();
        if name.ends_with(".txt") || mime == "text/plain" || mime.contains("text/plain") {
            let path = temp_path();
            let result = async {
                client
                    .download_file(&document, &path)
                    .await
                    .map_err(|e| e.to_string())?;
                let content = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(content)
            }
            .await;
            let _ = tokio::fs::remove_file(&path).await;
            if let Ok(content) = result {
                return ParsedCommand {
                    items: extract_batch_items(&content),
                    force: rerip || message.text().is_some_and(has_force_token),
                    storefront: None,
                    document: true,
                };
            }
        }
    }
    parse_text(
        message.text().unwrap_or_default(),
        reply.as_ref().and_then(IncomingMessage::text),
        rerip,
    )
    .unwrap_or(ParsedCommand {
        items: Vec::new(),
        force: rerip,
        storefront: None,
        document: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_and_flags_are_case_insensitive() {
        assert_eq!(command_name("/ALAC@bot 1"), Some("alac".into()));
        assert!(has_force_token("/alac --force 1"));
    }
}
