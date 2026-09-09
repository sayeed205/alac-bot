use bot::dashboard::{cancelable_job_ids, render, DashboardJob, DashboardSnapshot, JobPhase};

fn job(n: usize, allowed: bool) -> DashboardJob {
    DashboardJob {
        id: format!("j{n}"),
        requester_id: 10 + n as i64,
        requester_name: format!("Requester {n}"),
        header: format!("Album {n}"),
        phase: if n == 0 {
            JobPhase::Processing
        } else {
            JobPhase::Queued
        },
        queue_position: (n != 0).then_some(n as u64),
        cached: 2,
        ripped: 3,
        failed: 0,
        total: 10,
        percent: 50,
        is_cancel_allowed_for_viewer: allowed,
    }
}

#[test]
fn empty_dashboard_is_exact_and_pages_are_mobile_friendly() {
    let empty = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: None,
        jobs: vec![],
    };
    assert_eq!(render(&empty, 1, false).0, "<b>No active downloads.</b>");
    let jobs = (0..6).map(|n| job(n, n == 0)).collect();
    let snapshot = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: Some("healthy".into()),
        jobs,
    };
    let (text, keyboard) = render(&snapshot, 1, false);
    assert!(text.contains("Requester 0") && text.contains("Processing") && text.contains("#1"));
    assert!(text.contains("Page 1/2") && keyboard.is_some());
}

#[test]
fn cancel_controls_are_only_rendered_for_allowed_rows() {
    let snapshot = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: None,
        jobs: vec![job(0, true), job(1, false)],
    };
    let (_, keyboard) = render(&snapshot, 1, false);
    assert!(keyboard.is_some());
    assert_eq!(cancelable_job_ids(&snapshot, 1, false), vec!["j0"]);
    assert_eq!(cancelable_job_ids(&snapshot, 1, true), vec!["j0", "j1"]);
}
