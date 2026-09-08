//! The live status dashboard.  This module deliberately knows nothing about
//! ferogram; the small sink makes it usable by both Telegram and tests.
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::sync::Mutex;

pub type DashboardFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EditError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    FloodWait(Duration),
    Other(String),
}

pub trait DashboardSink: Send + Sync {
    fn send<'a>(
        &'a self,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> DashboardFuture<'a, i32>;
    fn edit<'a>(
        &'a self,
        id: i32,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> DashboardFuture<'a, ()>;
    fn delete<'a>(&'a self, id: i32) -> DashboardFuture<'a, ()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPhase {
    Processing,
    Queued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardJob {
    pub id: String,
    pub requester_id: i64,
    pub requester_name: String,
    pub header: String,
    pub phase: JobPhase,
    pub queue_position: Option<u64>,
    pub cached: u64,
    pub ripped: u64,
    pub failed: u64,
    pub total: u64,
    pub percent: u8,
    pub is_cancel_allowed_for_viewer: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardSnapshot {
    pub ripping_mode: String,
    pub mirror_health: Option<String>,
    pub jobs: Vec<DashboardJob>,
}

pub fn render(
    snapshot: &DashboardSnapshot,
    page: usize,
    viewer_is_admin: bool,
) -> (String, Option<ferogram::tl::enums::ReplyMarkup>) {
    if snapshot.jobs.is_empty() {
        return ("✅ <b>No active downloads.</b>".into(), None);
    }
    let pages = snapshot.jobs.len().div_ceil(5);
    let page = page.clamp(1, pages);
    let start = (page - 1) * 5;
    let health = snapshot.mirror_health.as_deref().unwrap_or("unknown");
    let mut text = format!(
        "📡 <b>Live downloads</b>\n<i>Mode: {} • Mirror: {}</i>\n",
        esc(&snapshot.ripping_mode),
        esc(health)
    );
    for job in snapshot.jobs.iter().skip(start).take(5) {
        let state = match job.phase {
            JobPhase::Processing => "▶ Processing".to_owned(),
            JobPhase::Queued => format!("#{}", job.queue_position.unwrap_or(0)),
        };
        text.push_str(&format!("\n<b>{}</b>\n👤 {} · <i>{}</i>\n{}% · ⚡ {} cached · 🎵 {} ripped · ⚠️ {} failed / {} total\n", esc(&job.header), esc(&job.requester_name), state, job.percent.min(100), job.cached, job.ripped, job.failed, job.total));
        // Actions are represented in the keyboard, never as a global control:
        // this keeps a shared dashboard safe in a group chat.
    }
    text.push_str(&format!(
        "\n<i>Page {page}/{pages} • {} active</i>",
        snapshot.jobs.len()
    ));
    let mut nav = Vec::new();
    if page > 1 {
        nav.push(ferogram::keyboard::Button::callback(
            "‹ Prev",
            format!("dashboard:prev:{page}").into_bytes(),
        ));
    }
    nav.push(ferogram::keyboard::Button::callback(
        "↻ Refresh",
        format!("dashboard:refresh:{page}").into_bytes(),
    ));
    if page < pages {
        nav.push(ferogram::keyboard::Button::callback(
            "Next ›",
            format!("dashboard:next:{page}").into_bytes(),
        ));
    }
    let mut keyboard_builder = ferogram::keyboard::InlineKeyboard::new();
    for job in snapshot.jobs.iter().skip(start).take(5) {
        if job.is_cancel_allowed_for_viewer || viewer_is_admin {
            keyboard_builder = keyboard_builder.row([ferogram::keyboard::Button::callback(
                format!("❌ Cancel · {}", truncate(&job.header, 18)),
                format!("cancel:{}", job.id).into_bytes(),
            )]);
        }
    }
    let keyboard = keyboard_builder.row(nav).into_markup();
    (text, Some(keyboard))
}

fn esc(s: &str) -> String {
    crate::html::escape(s)
}

fn truncate(s: &str, max: usize) -> String {
    let mut out = s.chars().take(max).collect::<String>();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

/// IDs for which the current viewer may receive a cancel button.  Kept
/// separate from Telegram markup so adapters can audit actions and test it.
pub fn cancelable_job_ids(
    snapshot: &DashboardSnapshot,
    page: usize,
    viewer_is_admin: bool,
) -> Vec<String> {
    let start = page.saturating_sub(1) * 5;
    snapshot
        .jobs
        .iter()
        .skip(start)
        .take(5)
        .filter(|job| job.is_cancel_allowed_for_viewer || viewer_is_admin)
        .map(|job| job.id.clone())
        .collect()
}

struct Entry {
    sink: Arc<dyn DashboardSink>,
    id: i32,
    page: usize,
    viewer_id: i64,
    viewer_is_admin: bool,
    snapshot: DashboardSnapshot,
    warning_sent: bool,
    empty_rendered: bool,
    /// Earliest time this entry's message may be edited again by a
    /// non-forced refresh (flood/coalescing per dashboard message).
    next_refresh_at: Option<tokio::time::Instant>,
}
pub struct DashboardManager {
    entries: Mutex<HashMap<i64, Entry>>,
}

/// Progress events can arrive many times per second; dashboards coalesce
/// them into at most one edit per interval per message.
const REFRESH_INTERVAL: Duration = Duration::from_millis(2500);

/// Recompute per-row cancel permission for a specific viewer. A shared group
/// dashboard renders only the actions that viewer is allowed to take
/// (requester or admin); actual authorization is re-checked on callback.
pub fn apply_viewer(snapshot: &mut DashboardSnapshot, viewer_id: i64, viewer_is_admin: bool) {
    for job in &mut snapshot.jobs {
        job.is_cancel_allowed_for_viewer = viewer_is_admin || viewer_id == job.requester_id;
    }
}

impl Default for DashboardManager {
    fn default() -> Self {
        Self::new()
    }
}
impl DashboardManager {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Sends the replacement before removing the previous dashboard. The
    /// snapshot is stored viewer-scoped so callbacks (page/refresh) render
    /// exactly what this viewer may act on.
    pub async fn open(
        &self,
        chat: i64,
        viewer_id: i64,
        viewer_is_admin: bool,
        sink: Arc<dyn DashboardSink>,
        snapshot: DashboardSnapshot,
    ) -> Result<i32, EditError> {
        let mut snapshot = snapshot;
        apply_viewer(&mut snapshot, viewer_id, viewer_is_admin);
        let (text, keyboard) = render(&snapshot, 1, viewer_is_admin);
        let new_id = sink.send(&text, keyboard).await?;
        let empty = snapshot.jobs.is_empty();
        let old = self.entries.lock().await.insert(
            chat,
            Entry {
                sink: Arc::clone(&sink),
                id: new_id,
                page: 1,
                viewer_id,
                viewer_is_admin,
                snapshot,
                warning_sent: false,
                empty_rendered: empty,
                next_refresh_at: None,
            },
        );
        if let Some(old) = old {
            let _ = old.sink.delete(old.id).await;
        }
        Ok(new_id)
    }

    /// Latest engine snapshot for a chat's dashboard, re-scoped to that
    /// dashboard's viewer. Used by the refresh callback so a stale dashboard
    /// can resynchronize without the event bridge.
    pub async fn refresh_entry_from(&self, chat: i64, snapshot: DashboardSnapshot) {
        let work = {
            let mut entries = self.entries.lock().await;
            entries.get_mut(&chat).map(|entry| {
                let mut viewed = snapshot;
                apply_viewer(&mut viewed, entry.viewer_id, entry.viewer_is_admin);
                entry.snapshot = viewed.clone();
                entry.next_refresh_at = None;
                let (text, keyboard) = render(&entry.snapshot, entry.page, entry.viewer_is_admin);
                (Arc::clone(&entry.sink), entry.id, text, keyboard)
            })
        };
        if let Some((sink, id, text, keyboard)) = work {
            let _ = sink.edit(id, &text, keyboard).await;
        }
    }
    pub async fn refresh_all(&self, snapshot: DashboardSnapshot) {
        self.refresh(snapshot, false).await;
    }

    /// Refresh every open dashboard. Terminal events pass `force = true` so
    /// queue changes (job finished/cancelled) are never suppressed by the
    /// coalescing window.
    pub async fn refresh(&self, snapshot: DashboardSnapshot, force: bool) {
        // Never retain the manager mutex across Telegram I/O: an edit can
        // block for a flood wait and callers must still be able to replace or
        // page a dashboard meanwhile.
        let work = {
            let mut entries = self.entries.lock().await;
            entries
                .iter_mut()
                .filter_map(|(chat, entry)| {
                    // Coalesce high-frequency progress events per message.
                    if !force
                        && entry
                            .next_refresh_at
                            .is_some_and(|at| at > tokio::time::Instant::now())
                    {
                        return None;
                    }
                    let mut viewed = snapshot.clone();
                    apply_viewer(&mut viewed, entry.viewer_id, entry.viewer_is_admin);
                    entry.snapshot = viewed.clone();
                    if entry.snapshot.jobs.is_empty() {
                        if entry.empty_rendered {
                            return None;
                        }
                        entry.empty_rendered = true;
                    } else {
                        entry.empty_rendered = false;
                    }
                    entry.next_refresh_at = Some(tokio::time::Instant::now() + REFRESH_INTERVAL);
                    let (text, keyboard) =
                        render(&entry.snapshot, entry.page, entry.viewer_is_admin);
                    Some((
                        *chat,
                        Arc::clone(&entry.sink),
                        entry.id,
                        text,
                        keyboard,
                        entry.warning_sent,
                    ))
                })
                .collect::<Vec<_>>()
        };

        for (chat, sink, id, text, keyboard, warning_was_sent) in work {
            match sink.edit(id, &text, keyboard).await {
                Ok(()) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.warning_sent = false;
                    }
                }
                Err(EditError::FloodWait(wait)) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.warning_sent = true;
                        // Suppress further edits until the flood deadline.
                        entry.next_refresh_at =
                            Some(tokio::time::Instant::now() + wait + REFRESH_INTERVAL);
                    }
                    if !warning_was_sent {
                        let _ = sink
                            .send(
                                "⚠️ Live status updates are temporarily paused; refresh resumes shortly.",
                                None,
                            )
                            .await;
                        tracing::warn!("dashboard updates throttled");
                    }
                }
                Err(EditError::Other(error)) => tracing::warn!(%error, "dashboard update failed"),
            }
        }
    }
    pub async fn page(&self, chat: i64, page: usize) {
        let work = {
            let mut entries = self.entries.lock().await;
            entries.get_mut(&chat).map(|entry| {
                entry.page = page;
                let (text, keyboard) = render(&entry.snapshot, page, entry.viewer_is_admin);
                (Arc::clone(&entry.sink), entry.id, text, keyboard)
            })
        };
        if let Some((sink, id, text, keyboard)) = work {
            let _ = sink.edit(id, &text, keyboard).await;
        }
    }
}
