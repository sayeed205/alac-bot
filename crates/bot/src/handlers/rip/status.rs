//! Status-message lifecycle and exact HTML rendering.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use ferogram::{
    keyboard::{Button, InlineKeyboard},
    InputMessage, PeerRef,
};
use tokio::sync::Mutex;

pub type BoxStatusFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// Telegram-safe default for detailed status edits. Callers may force a
/// terminal update, but routine progress never exceeds one edit per 10s.
const DEFAULT_EDIT_INTERVAL: Duration = Duration::from_secs(10);

/// Small Telegram surface so rendering/throttling can be tested without a
/// network client.
pub trait StatusSink: Send + Sync {
    fn send<'a>(
        &'a self,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> BoxStatusFuture<'a, i32>;
    fn edit<'a>(
        &'a self,
        message_id: i32,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> BoxStatusFuture<'a, ()>;
}

pub struct TelegramStatusSink {
    pub client: ferogram::Client,
    pub peer: PeerRef,
}

impl StatusSink for TelegramStatusSink {
    fn send<'a>(
        &'a self,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> BoxStatusFuture<'a, i32> {
        Box::pin(async move {
            let mut input = InputMessage::html(text);
            if let Some(k) = keyboard {
                input = input.reply_markup(k);
            }
            self.client
                .send_message(self.peer.clone(), input)
                .await
                .map(|m| m.id())
                .map_err(|e| e.to_string())
        })
    }
    fn edit<'a>(
        &'a self,
        message_id: i32,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> BoxStatusFuture<'a, ()> {
        Box::pin(async move {
            let mut input = InputMessage::html(text);
            if let Some(k) = keyboard {
                input = input.reply_markup(k);
            }
            self.client
                .edit_message(self.peer.clone(), message_id, input)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProgressState {
    pub header: String,
    pub total: usize,
    pub cached: usize,
    pub ripped: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cache_only: bool,
    pub group: bool,
    pub active_download: Option<String>,
    pub active_upload: Option<String>,
}

impl ProgressState {
    pub fn semantic_view(&self) -> crate::presentation::RipStatusView {
        let completed = self.cached + self.ripped + self.failed + self.skipped;
        let state = if self.active_upload.is_some() {
            "Uploading"
        } else if self.active_download.is_some() {
            "Downloading"
        } else {
            "Processing"
        };
        crate::presentation::RipStatusView {
            state,
            target: self.header.clone(),
            completed,
            total: self.total,
            cached: self.cached,
            ripped: self.ripped,
            skipped: self.skipped,
            failed: self.failed,
            activity: self
                .active_upload
                .clone()
                .or_else(|| self.active_download.clone()),
        }
    }
}

pub fn render_progress(s: &ProgressState, override_text: Option<&str>) -> String {
    let completed = s.cached + s.ripped + s.failed + s.skipped;
    let state = if s.active_upload.is_some() {
        "Uploading"
    } else if s.active_download.is_some() {
        "Downloading"
    } else {
        "Processing"
    };
    let pct = if s.total == 0 {
        0.0
    } else {
        (completed as f64 / s.total as f64) * 100.0
    };
    let mode = if s.cache_only { "#Cache" } else { "#Rip" };
    let mut out = format!(
        "<b>{}</b><br/>┃ <code>{}</code><br/>┠ Status: {}<br/>┠ Processed: {} of {} tracks",
        s.header,
        crate::presentation::box_progress_bar(pct),
        state,
        completed,
        s.total,
    );
    if s.skipped > 0 {
        out.push_str(&format!("<br/>┠ ⚠️ {} skipped", s.skipped));
    }
    if s.failed > 0 {
        out.push_str(&format!("<br/>┠ ⚠️ {} failed", s.failed));
    }
    if let Some(download) = &s.active_download {
        out.push_str("<br/>┠ ");
        out.push_str(download);
    }
    if let Some(upload) = &s.active_upload {
        out.push_str("<br/>┠ ");
        out.push_str(upload);
    }
    if s.active_download.is_none() && s.active_upload.is_none() {
        if let Some(override_text) = override_text {
            out.push_str(&format!("<br/>┠ Current: {override_text}"));
        }
    }
    out.push_str(&format!("<br/>┠ Mode: {mode}"));
    if s.group && !s.cache_only {
        out.push_str("<br/>┠ <i>Files delivered to your private chat.</i>");
    }
    out.push_str("<br/>┖ Progress updates here");
    out
}

pub fn cancel_keyboard(job_id: &str) -> ferogram::tl::enums::ReplyMarkup {
    InlineKeyboard::new()
        .row([Button::callback(
            "Cancel download",
            crate::interaction::TelegramAction::Cancel {
                job_id: job_id.to_owned(),
            }
            .encode()
            .into_bytes(),
        )])
        .into_markup()
}

#[derive(Clone)]
pub struct StatusEditor {
    sink: Arc<dyn StatusSink>,
    message_id: i32,
    job_id: String,
    last_text: Arc<Mutex<String>>,
    last_edit: Arc<Mutex<Option<tokio::time::Instant>>>,
    editing: Arc<Mutex<bool>>,
    blocked: Arc<Mutex<bool>>,
}

impl StatusEditor {
    pub fn new(sink: Arc<dyn StatusSink>, message_id: i32, job_id: impl Into<String>) -> Self {
        Self {
            sink,
            message_id,
            job_id: job_id.into(),
            last_text: Arc::new(Mutex::new(String::new())),
            last_edit: Arc::new(Mutex::new(None)),
            editing: Arc::new(Mutex::new(false)),
            blocked: Arc::new(Mutex::new(false)),
        }
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub async fn blocked(&self) -> bool {
        *self.blocked.lock().await
    }

    /// First edit is immediate; later edits are throttled for ten seconds.
    /// A forced edit bypasses the throttle. Only one edit may be in flight.
    pub async fn update(&self, text: String, force: bool, terminal: bool) -> bool {
        if *self.blocked.lock().await && !terminal {
            return false;
        }
        {
            let mut editing = self.editing.lock().await;
            if *editing {
                return false;
            }
            let same = *self.last_text.lock().await == text;
            if same {
                return false;
            }
            let elapsed = self
                .last_edit
                .lock()
                .await
                .map(|t| t.elapsed())
                .unwrap_or(DEFAULT_EDIT_INTERVAL);
            if !force && elapsed < DEFAULT_EDIT_INTERVAL {
                return false;
            }
            *editing = true;
        }
        let previous_text = self.last_text.lock().await.clone();
        *self.last_text.lock().await = text.clone();
        *self.last_edit.lock().await = Some(tokio::time::Instant::now());
        let keyboard = (!terminal).then(|| cancel_keyboard(&self.job_id));
        let result = self.sink.edit(self.message_id, &text, keyboard).await;
        *self.editing.lock().await = false;
        if result.is_err() {
            // A transient edit failure must not permanently freeze progress.
            // Restore the previous text so the next update can retry it.
            *self.last_text.lock().await = previous_text;
            if terminal {
                *self.blocked.lock().await = true;
                let _ = self.sink.send(&text, None).await;
            }
        }
        result.is_ok()
    }

    pub async fn final_text(&self, text: String) {
        let _ = self.update(text, true, true).await;
    }
}

pub fn cancelled_text(target: &str, by: &str, processed: usize, total: usize) -> String {
    format!("! <b>Download cancelled</b><br/><br/><blockquote>• <b>Target:</b> {target}<br/>• <b>Cancelled by:</b> <b>{}</b><br/>• <b>Progress when cancelled:</b> <code>{processed}/{total} tracks processed</code></blockquote>", escape(by))
}

#[derive(Debug, Clone)]
pub struct SummaryInput {
    pub target: String,
    pub total: usize,
    pub cached: usize,
    pub ripped: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
    pub elapsed: String,
    pub cache_only: bool,
    pub group: bool,
    pub capped: usize,
    pub cap_limit: u32,
}

pub fn final_summary(s: &SummaryInput) -> String {
    let mut out = if s.cache_only {
        format!("✓ <b>Caching complete</b><br/><br/><blockquote>• <b>Target:</b> {}<br/>• <b>Total tracks:</b> <code>{}</code><br/>• <b>Seeded to dump:</b> <code>{}</code> new · <code>{}</code> already cached<br/>{}• <b>Time elapsed:</b> <code>{}s</code><br/>• <b>Destination:</b> Dump channel and database</blockquote>", s.target, s.total, s.ripped, s.cached, failed_line(&s.failed), s.elapsed)
    } else if s.cached == 0 && s.ripped == 0 && s.skipped > 0 {
        format!("! <b>No cached tracks available</b><br/><br/><blockquote>• <b>Target:</b> {}<br/>• <b>Total requested:</b> <code>{}</code><br/>• <b>Skipped (uncached):</b> <code>{}</code><br/>• <b>Next step:</b> Live ripping is temporarily unavailable.</blockquote>", s.target, s.total, s.skipped)
    } else {
        format!("✓ <b>Download complete</b><br/><br/><blockquote>• <b>Target:</b> {}<br/>• <b>Total tracks:</b> <code>{}</code><br/>• <b>Delivered:</b> <code>{}</code> cached · <code>{}</code> ripped<br/>{}{}• <b>Time elapsed:</b> <code>{}s</code></blockquote>", s.target, s.total, s.cached, s.ripped, if s.skipped > 0 { format!("• <b>Skipped (uncached):</b> <code>{}</code><br/>", s.skipped) } else { String::new() }, failed_line(&s.failed), s.elapsed)
    };
    if s.capped > 0 {
        out.push_str(&format!(
            "<br/><i>Queue capped to {} tracks by the settings limit.</i>",
            s.cap_limit
        ));
    }
    if s.group && !s.cache_only {
        out.push_str("<br/><i>All songs have been delivered to your private chat.</i>");
    }
    if !s.failed.is_empty() {
        out.push_str("<br/><br/><b>Issues / Failures:</b><br/>");
        for (id, error) in s.failed.iter().take(5) {
            out.push_str(&format!(
                "• <code>{}</code>: {}<br/>",
                escape(id),
                escape(error)
            ));
        }
        if s.failed.len() > 5 {
            out.push_str(&format!("<i>...and {} more</i>", s.failed.len() - 5));
        }
    }
    out
}

fn failed_line(failed: &[(String, String)]) -> String {
    if failed.is_empty() {
        String::new()
    } else {
        format!("• <b>Failed:</b> <code>{}</code><br/>", failed.len())
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_box_follows_status_policy() {
        let text = render_progress(
            &ProgressState {
                header: "Track ID: <code>1</code>".into(),
                total: 2,
                cached: 1,
                group: true,
                ..Default::default()
            },
            None,
        );
        // Box shape: title, bar line under ┃, status/processed lines.
        assert!(text.contains("┃ <code>"));
        assert!(text.contains("┠ Status: Processing"));
        assert!(text.contains("┠ Processed: 1 of 2 tracks"));
        assert!(text.contains("┠ Mode: #Rip"));
        assert!(text.contains("<i>Files delivered to your private chat.</i>"));
        assert!(text.ends_with("┖ Progress updates here"));
    }

    #[test]
    fn progress_box_reports_active_upload_state() {
        let text = render_progress(
            &ProgressState {
                header: "Album: <b>X</b>".into(),
                total: 4,
                cached: 2,
                active_upload: Some("⬆️ <b>Uploading:</b> <code>45%</code>".into()),
                ..Default::default()
            },
            None,
        );
        assert!(text.contains("┠ Status: Uploading"));
        assert!(text.contains("┠ ⬆️ <b>Uploading:</b> <code>45%</code>"));
        assert!(!text.contains("┠ Mode: #Cache"));
    }

    #[test]
    fn final_summary_has_cap_note_group_note_and_first_five_failures() {
        let summary = SummaryInput {
            target: "Batch: <b>7 tracks</b>".into(),
            total: 7,
            cached: 1,
            ripped: 1,
            skipped: 0,
            failed: (0..6).map(|i| (i.to_string(), "bad".into())).collect(),
            elapsed: "1.0".into(),
            cache_only: false,
            group: true,
            capped: 1,
            cap_limit: 5,
        };
        let text = final_summary(&summary);
        assert!(text.contains("Queue capped to 5 tracks by the settings limit."));
        assert!(text.contains("All songs have been delivered to your private chat."));
        assert!(text.contains("...and 1 more"));
    }
}
