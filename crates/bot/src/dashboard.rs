//! The live status dashboard.  This module deliberately knows nothing about
//! ferogram; the small sink makes it usable by both Telegram and tests.
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::sync::Mutex;

pub type DashboardFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EditError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    FloodWait(Duration),
    NotModified,
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
    /// Current per-track activity, if the pipeline is downloading/uploading.
    /// Values are trusted presentation fragments with dynamic text escaped.
    pub downloading: Option<String>,
    pub uploading: Option<String>,
    pub is_cancel_allowed_for_viewer: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardSnapshot {
    pub ripping_mode: String,
    pub mirror_health: Option<String>,
    pub current_activity: Option<String>,
    pub jobs: Vec<DashboardJob>,
}

pub fn render(
    snapshot: &DashboardSnapshot,
    page: usize,
    _viewer_is_admin: bool,
) -> (String, Option<ferogram::tl::enums::ReplyMarkup>) {
    if snapshot.jobs.is_empty() {
        return ("<b>No active downloads.</b>".into(), None);
    }
    let pages = snapshot.jobs.len().div_ceil(5);
    let page = page.clamp(1, pages);
    let start = (page - 1) * 5;
    let health = snapshot.mirror_health.as_deref().unwrap_or("unknown");
    let mut text = format!(
        "<b>Live downloads</b>\n<i>Mode: {} · Mirror: {}</i>\n",
        esc(&snapshot.ripping_mode),
        esc(health)
    );
    if let Some(activity) = snapshot.current_activity.as_deref() {
        text.push_str(&format!("<b>Activity:</b> {activity}\n"));
    }
    for (offset, job) in snapshot.jobs.iter().skip(start).take(5).enumerate() {
        let number = start + offset + 1;
        let state = match job.phase {
            JobPhase::Processing => "Processing".to_owned(),
            JobPhase::Queued => job.queue_position.map_or_else(
                || "Queued".to_owned(),
                |position| format!("Queued · position #{position}"),
            ),
        };
        let pct = job.percent.min(100) as f64;
        let mut lines = vec![
            format!("<i>{number}.</i> {}", job.header),
            format!(
                "┃ <code>{}</code>",
                crate::presentation::box_progress_bar(pct)
            ),
        ];
        if job.phase != JobPhase::Processing {
            lines.push(format!("┝ Status: {}", esc(&state)));
        }
        lines.push(format!(
            "┝ Processed: {} of {} tracks",
            (job.cached + job.ripped + job.failed).min(job.total),
            job.total
        ));
        lines.push(format!(
            "┝ Cache: {} hit · {} ripped · {} failed",
            job.cached, job.ripped, job.failed
        ));
        lines.push(format!("┝ Cancel: /cancel_{}", job.id));
        let by_mention = if job.requester_name.starts_with('@') {
            let handle = job.requester_name.trim_start_matches('@');
            format!("<a href=\"https://t.me/{handle}\">@{handle}</a>")
        } else if job.requester_id > 0 {
            format!(
                "<a href=\"tg://user?id={}\">{}</a>",
                job.requester_id,
                esc(&job.requester_name)
            )
        } else {
            esc(&job.requester_name)
        };
        lines.push(format!("┕ By: {by_mention}"));
        text.push_str(&format!("\n{}<br/>", lines.join("<br/>")));
    }
    text.push_str(&format!(
        "\n<i>Page {page}/{pages} • {} active</i>",
        snapshot.jobs.len()
    ));
    let mut nav = Vec::new();
    if page > 1 {
        nav.push(ferogram::keyboard::Button::callback(
            "Previous",
            crate::interaction::TelegramAction::Dashboard {
                action: crate::interaction::DashboardAction::Previous,
                page,
            }
            .encode()
            .into_bytes(),
        ));
    }
    nav.push(ferogram::keyboard::Button::callback(
        "Refresh",
        crate::interaction::TelegramAction::Dashboard {
            action: crate::interaction::DashboardAction::Refresh,
            page,
        }
        .encode()
        .into_bytes(),
    ));
    if page < pages {
        nav.push(ferogram::keyboard::Button::callback(
            "Next",
            crate::interaction::TelegramAction::Dashboard {
                action: crate::interaction::DashboardAction::Next,
                page,
            }
            .encode()
            .into_bytes(),
        ));
    }
    let keyboard = ferogram::keyboard::InlineKeyboard::new()
        .row(nav)
        .into_markup();
    (text, Some(keyboard))
}

fn esc(s: &str) -> String {
    crate::html::escape(s)
}

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
    empty_rendered: bool,
    /// Earliest time this entry's message may be edited again by a
    /// non-forced refresh (flood/coalescing per dashboard message).
    next_refresh_at: Option<tokio::time::Instant>,
    /// Telegram flood deadline. Unlike the normal coalescing window this
    /// deadline is never bypassed by a forced refresh.
    flood_until: Option<tokio::time::Instant>,
}
pub struct DashboardManager {
    entries: Mutex<HashMap<i64, Entry>>,
}

/// Progress events can arrive many times per second; dashboards coalesce
/// them into at most one edit per interval per message. Ten seconds is the
/// default safety floor for Telegram traffic; terminal transitions still use
/// forced edits when they are not inside a flood-wait window.
const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

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

    /// Whether this chat already has the single managed dashboard message.
    pub async fn contains(&self, chat: i64) -> bool {
        self.entries.lock().await.contains_key(&chat)
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
                empty_rendered: empty,
                next_refresh_at: None,
                flood_until: None,
            },
        );
        if let Some(old) = old {
            let _ = old.sink.delete(old.id).await;
        }
        Ok(new_id)
    }

    /// Replace the dashboard message for a chat while preserving its
    /// viewer/sink configuration. This is used when a new job is created so
    /// the dashboard has a fresh message rather than silently editing the
    /// previous snapshot in place.
    pub async fn replace_entry_from(
        &self,
        chat: i64,
        snapshot: DashboardSnapshot,
    ) -> Result<(), EditError> {
        let Some((viewer_id, viewer_is_admin, sink)) = ({
            let entries = self.entries.lock().await;
            entries.get(&chat).map(|entry| {
                (
                    entry.viewer_id,
                    entry.viewer_is_admin,
                    Arc::clone(&entry.sink),
                )
            })
        }) else {
            return Ok(());
        };

        self.open(chat, viewer_id, viewer_is_admin, sink, snapshot)
            .await
            .map(|_| ())
    }

    /// Latest engine snapshot for a chat's dashboard, re-scoped to that
    /// dashboard's viewer. Used by the refresh callback so a stale dashboard
    /// can resynchronize without the event bridge.
    pub async fn refresh_entry_from(&self, chat: i64, snapshot: DashboardSnapshot) {
        let work = {
            let mut entries = self.entries.lock().await;
            entries.get_mut(&chat).and_then(|entry| {
                let mut viewed = snapshot;
                apply_viewer(&mut viewed, entry.viewer_id, entry.viewer_is_admin);
                entry.snapshot = viewed.clone();
                if entry
                    .flood_until
                    .is_some_and(|at| at > tokio::time::Instant::now())
                {
                    return None;
                }
                entry.next_refresh_at = None;
                let (text, keyboard) = render(&entry.snapshot, entry.page, entry.viewer_is_admin);
                Some((Arc::clone(&entry.sink), entry.id, text, keyboard))
            })
        };
        if let Some((sink, id, text, keyboard)) = work {
            // User-driven refreshes use the same flood deadline as background
            // edits, so a Refresh tap cannot create a second flood episode.
            match sink.edit(id, &text, keyboard).await {
                Ok(()) | Err(EditError::NotModified) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.flood_until = None;
                    }
                }
                Err(EditError::FloodWait(wait)) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        let until = tokio::time::Instant::now() + wait;
                        entry.flood_until = Some(until);
                        entry.next_refresh_at = Some(until);
                    }
                }
                Err(EditError::Other(_)) => {}
            }
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
                    // A flood deadline is stronger than `force`: terminal
                    // events must not immediately repeat a request Telegram
                    // has already rejected.
                    if entry
                        .flood_until
                        .is_some_and(|at| at > tokio::time::Instant::now())
                    {
                        return None;
                    }
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
                    Some((*chat, Arc::clone(&entry.sink), entry.id, text, keyboard))
                })
                .collect::<Vec<_>>()
        };

        for (chat, sink, id, text, keyboard) in work {
            match sink.edit(id, &text, keyboard).await {
                Ok(()) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.flood_until = None;
                    }
                }
                Err(EditError::NotModified) => {
                    // Telegram treats an identical edit as a benign
                    // condition, not a dashboard failure.
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.flood_until = None;
                    }
                }
                Err(EditError::FloodWait(wait)) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        // Suppress further edits until the flood deadline.
                        let until = tokio::time::Instant::now() + wait;
                        entry.flood_until = Some(until);
                        entry.next_refresh_at = Some(until);
                    }
                    tracing::warn!(chat_id = chat, ?wait, "dashboard updates throttled");
                }
                Err(EditError::Other(error)) => tracing::warn!(%error, "dashboard update failed"),
            }
        }
    }
    pub async fn page(&self, chat: i64, page: usize) {
        let work = {
            let mut entries = self.entries.lock().await;
            entries.get_mut(&chat).and_then(|entry| {
                // Clamp to the real page range so repeated Next taps from
                // the last page stay on the last page.
                if entry
                    .flood_until
                    .is_some_and(|at| at > tokio::time::Instant::now())
                {
                    return None;
                }
                let pages = entry.snapshot.jobs.len().div_ceil(5).max(1);
                let page = page.clamp(1, pages);
                entry.page = page;
                let (text, keyboard) = render(&entry.snapshot, page, entry.viewer_is_admin);
                Some((Arc::clone(&entry.sink), entry.id, text, keyboard))
            })
        };
        if let Some((sink, id, text, keyboard)) = work {
            // Successful user-driven edits close a flood episode, matching
            // refresh_entry_from. Flood failures set the same deadline so
            // repeated page taps remain cheap and bounded.
            match sink.edit(id, &text, keyboard).await {
                Ok(()) | Err(EditError::NotModified) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        entry.flood_until = None;
                    }
                }
                Err(EditError::FloodWait(wait)) => {
                    if let Some(entry) = self.entries.lock().await.get_mut(&chat) {
                        let until = tokio::time::Instant::now() + wait;
                        entry.flood_until = Some(until);
                        entry.next_refresh_at = Some(until);
                    }
                }
                Err(EditError::Other(_)) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_job_headers_render_as_html_and_plain_button_labels() {
        let snapshot = DashboardSnapshot {
            jobs: vec![DashboardJob {
                id: "job-1".into(),
                requester_id: 7,
                requester_name: "Alice".into(),
                header: "Album: <b>3 Originals</b> by <b>Rick Astley</b>".into(),
                phase: JobPhase::Processing,
                queue_position: None,
                cached: 0,
                ripped: 0,
                failed: 0,
                total: 1,
                percent: 0,
                downloading: None,
                uploading: None,
                is_cancel_allowed_for_viewer: true,
            }],
            ..DashboardSnapshot::default()
        };

        let (text, markup) = render(&snapshot, 1, false);
        assert!(text.contains("<i>1.</i> Album: <b>3 Originals</b> by <b>Rick Astley</b>"));
        assert!(text.contains("Album: <b>3 Originals</b> by <b>Rick Astley</b>"));
        assert!(text.contains("/cancel_job-1"));
        assert!(!text.contains("&lt;b&gt;"));
        assert!(markup.is_some());
    }

    #[test]
    fn queued_jobs_show_an_explicit_queue_state_and_position() {
        let snapshot = DashboardSnapshot {
            jobs: vec![DashboardJob {
                id: "job-queued".into(),
                requester_id: 7,
                requester_name: "Alice".into(),
                header: "Track: <b>Song</b>".into(),
                phase: JobPhase::Queued,
                queue_position: Some(2),
                cached: 0,
                ripped: 0,
                failed: 0,
                total: 1,
                percent: 0,
                downloading: None,
                uploading: None,
                is_cancel_allowed_for_viewer: false,
            }],
            ..DashboardSnapshot::default()
        };
        let (text, _) = render(&snapshot, 1, false);
        assert!(text.contains("<i>1.</i> Track: <b>Song</b>"));
        assert!(text.contains("Queued · position #2"));
        assert!(!text.contains("<i>#2</i>"));
    }

    #[test]
    fn dashboard_numbers_entries_across_pages() {
        let jobs = (1..=6)
            .map(|number| DashboardJob {
                id: format!("job-{number}"),
                requester_id: 7,
                requester_name: "Alice".into(),
                header: format!("Track {number}"),
                phase: JobPhase::Processing,
                queue_position: None,
                cached: 0,
                ripped: 0,
                failed: 0,
                total: 1,
                percent: 0,
                downloading: None,
                uploading: None,
                is_cancel_allowed_for_viewer: false,
            })
            .collect();
        let snapshot = DashboardSnapshot {
            jobs,
            ..DashboardSnapshot::default()
        };

        let (text, _) = render(&snapshot, 2, false);
        assert!(text.contains("<i>6.</i> Track 6"));
        assert!(!text.contains("<i>1.</i> Track 1"));
    }

    #[test]
    fn dashboard_shows_current_download_and_upload_at_the_end() {
        let snapshot = DashboardSnapshot {
            jobs: vec![DashboardJob {
                id: "job-activity".into(),
                requester_id: 7,
                requester_name: "Alice".into(),
                header: "Track".into(),
                phase: JobPhase::Processing,
                queue_position: None,
                cached: 0,
                ripped: 0,
                failed: 0,
                total: 1,
                percent: 0,
                downloading: Some("<b>Song - Artist:</b> <code>1 MB</code>".into()),
                uploading: Some("<i>Song - Artist</i>".into()),
                is_cancel_allowed_for_viewer: true,
            }],
            current_activity: Some("⬇️ Downloading: <b>Song</b>".into()),
            ..DashboardSnapshot::default()
        };

        let (text, _) = render(&snapshot, 1, false);
        assert!(text.contains("<b>Activity:</b> ⬇️ Downloading: <b>Song</b>"));
        assert!(text.contains("/cancel_job-activity"));
    }
}
