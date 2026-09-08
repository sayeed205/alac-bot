//! `/cancel` and inline cancel authorization (oracle `commands-rip.ts:323-432`).

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::status::{cancelled_text, StatusEditor};

#[derive(Clone)]
pub struct RipJobHandle {
    pub id: String,
    pub chat_id: i64,
    pub requester_id: i64,
    pub target: String,
    pub total: usize,
    pub processed: Arc<Mutex<usize>>,
    pub controller: CancellationToken,
    pub status: Arc<StatusEditor>,
    pub completed: Arc<Mutex<bool>>,
}

pub type ActiveJobs = Arc<Mutex<std::collections::HashMap<String, RipJobHandle>>>;

pub fn new_jobs() -> ActiveJobs {
    Arc::new(Mutex::new(std::collections::HashMap::new()))
}

pub async fn cancel_inline(
    jobs: &ActiveJobs,
    job_id: &str,
    caller_id: i64,
    is_admin: bool,
    caller_name: &str,
) -> CancelResult {
    let job = jobs.lock().await.get(job_id).cloned();
    let Some(job) = job else {
        return CancelResult::Expired;
    };
    if *job.completed.lock().await {
        return CancelResult::Expired;
    }
    if !is_admin && caller_id != job.requester_id {
        return CancelResult::Unauthorized;
    }
    job.controller.cancel();
    *job.completed.lock().await = true;
    jobs.lock().await.remove(job_id);
    let processed = *job.processed.lock().await;
    job.status
        .final_text(cancelled_text(
            &job.target,
            caller_name,
            processed,
            job.total,
        ))
        .await;
    CancelResult::Cancelled
}

pub async fn cancel_command(
    jobs: &ActiveJobs,
    chat_id: i64,
    caller_id: i64,
    is_admin: bool,
    caller_name: &str,
) -> Option<RipJobHandle> {
    let job = jobs
        .lock()
        .await
        .values()
        .find(|job| job.chat_id == chat_id && (is_admin || job.requester_id == caller_id))
        .cloned()?;
    if *job.completed.lock().await {
        return None;
    }
    job.controller.cancel();
    *job.completed.lock().await = true;
    jobs.lock().await.remove(&job.id);
    let processed = *job.processed.lock().await;
    job.status
        .final_text(cancelled_text(
            &job.target,
            caller_name,
            processed,
            job.total,
        ))
        .await;
    Some(job)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelResult {
    Cancelled,
    Unauthorized,
    Expired,
}

pub const NO_ACTIVE: &str = "ℹ️ <b>No active download to cancel in this chat.</b>";
pub const COMMAND_ACK: &str = "🛑 <b>Download has been cancelled.</b>";
pub const CALLBACK_ACK: &str = "🛑 Download cancelled.";
pub const CALLBACK_UNAUTHORIZED: &str =
    "⛔ Only the person who requested this download or an admin can cancel it.";
pub const CALLBACK_EXPIRED: &str = "⚠️ This download has already completed or expired.";
