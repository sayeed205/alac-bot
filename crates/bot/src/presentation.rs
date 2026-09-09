//! Shared Telegram presentation policy.
//!
//! Handlers provide semantic facts; this module owns the small, calm English
//! surface users see.  The policy deliberately keeps emoji out of neutral
//! content and routine navigation.

use ferogram::tl::enums::ReplyMarkup;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackKind {
    Neutral,
    Success,
    Warning,
    Error,
    InProgress,
}

#[derive(Debug, Clone)]
pub struct TelegramMessage {
    pub html: String,
    pub keyboard: Option<ReplyMarkup>,
}

/// Semantic job state shared by the detailed card and dashboard adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RipStatusView {
    pub state: &'static str,
    pub target: String,
    pub completed: usize,
    pub total: usize,
    pub cached: usize,
    pub ripped: usize,
    pub skipped: usize,
    pub failed: usize,
    pub activity: Option<String>,
}

impl TelegramMessage {
    pub fn new(html: impl Into<String>) -> Self {
        Self {
            html: html.into(),
            keyboard: None,
        }
    }

    pub fn with_keyboard(mut self, keyboard: ReplyMarkup) -> Self {
        self.keyboard = Some(keyboard);
        self
    }
}

/// Render a heading with at most one semantic leading symbol.  Symbols are
/// intentionally absent from neutral content and all button labels.
pub fn heading(kind: FeedbackKind, title: &str) -> String {
    let symbol = match kind {
        FeedbackKind::Neutral => "",
        FeedbackKind::Success => "✓ ",
        FeedbackKind::Warning => "! ",
        FeedbackKind::Error => "× ",
        FeedbackKind::InProgress => "… ",
    };
    format!("{symbol}<b>{title}</b>")
}

pub fn escape(value: &str) -> String {
    crate::html::escape(value)
}

pub fn action_label(action: &str, target: Option<&str>) -> String {
    match target {
        Some(target) if !target.is_empty() => format!("{action} · {target}"),
        _ => action.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_semantic_and_neutral_is_plain() {
        assert_eq!(heading(FeedbackKind::Neutral, "Search"), "<b>Search</b>");
        assert_eq!(
            heading(FeedbackKind::Success, "Complete"),
            "✓ <b>Complete</b>"
        );
        assert_eq!(heading(FeedbackKind::Warning, "Paused"), "! <b>Paused</b>");
    }

    #[test]
    fn action_labels_use_words() {
        assert_eq!(action_label("Cancel download", None), "Cancel download");
        assert_eq!(action_label("Cached", Some("Song")), "Cached · Song");
    }
}
