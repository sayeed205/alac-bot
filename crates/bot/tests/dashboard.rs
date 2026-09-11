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
        downloading: None,
        uploading: None,
        is_cancel_allowed_for_viewer: allowed,
    }
}

#[test]
fn empty_dashboard_is_exact_and_pages_are_mobile_friendly() {
    let empty = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: None,
        current_download: None,
        current_upload: None,
        jobs: vec![],
    };
    assert_eq!(render(&empty, 1, false).0, "<b>No active downloads.</b>");
    let jobs = (0..6).map(|n| job(n, n == 0)).collect();
    let snapshot = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: Some("healthy".into()),
        current_download: None,
        current_upload: None,
        jobs,
    };
    let (text, keyboard) = render(&snapshot, 1, false);
    assert!(
        text.contains("Requester 0") && !text.contains("Status: Processing") && text.contains("1.")
    );
    assert!(text.contains("Status: Queued") && text.contains("Page 1/2") && keyboard.is_some());
}

#[test]
fn cancel_controls_are_only_rendered_for_allowed_rows() {
    let snapshot = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: None,
        current_download: None,
        current_upload: None,
        jobs: vec![job(0, true), job(1, false)],
    };
    let (text, keyboard) = render(&snapshot, 1, false);
    assert!(keyboard.is_some());
    assert!(text.contains("/cancel_j0"));
    assert!(text.contains("/cancel_j1"));
    assert_eq!(cancelable_job_ids(&snapshot, 1, false), vec!["j0"]);
    assert_eq!(cancelable_job_ids(&snapshot, 1, true), vec!["j0", "j1"]);
}

#[test]
fn header_renders_both_lane_lines_independently() {
    let both = DashboardSnapshot {
        ripping_mode: "sequential".into(),
        mirror_health: None,
        current_download: Some("<b>Song</b> <code>1 MB</code>".into()),
        current_upload: Some("<b>Album.zip</b>".into()),
        jobs: vec![job(0, true)],
    };
    let (text, _) = render(&both, 1, false);
    assert!(text.contains("<b>⬇️ Downloading:</b> <b>Song</b> <code>1 MB</code>"));
    assert!(text.contains("<b>⬆️ Uploading:</b> <b>Album.zip</b>"));

    let download_only = DashboardSnapshot {
        current_download: Some("<b>Song</b>".into()),
        current_upload: None,
        ..both.clone()
    };
    let (text, _) = render(&download_only, 1, false);
    assert!(text.contains("<b>⬇️ Downloading:</b> <b>Song</b>"));
    assert!(!text.contains("⬆️"));

    let idle = DashboardSnapshot {
        current_download: None,
        current_upload: None,
        jobs: vec![job(0, true)],
        ..both
    };
    let (text, _) = render(&idle, 1, false);
    assert!(!text.contains("⬇️"));
    assert!(!text.contains("⬆️"));
}
