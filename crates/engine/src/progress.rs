//! Progress formatting helpers. Port of `src/utils/progress.ts`.

/// Render a Unicode block progress bar, e.g. `[██████░░░░░░] 50%`.
pub fn render_progress_bar(current: u64, total: u64, length: usize) -> String {
    if total == 0 {
        return format!("[{}] 0%", "░".repeat(length));
    }
    let fraction = (current as f64 / total as f64).clamp(0.0, 1.0);
    let filled_count = (fraction * length as f64).round() as usize;
    let empty_count = length.saturating_sub(filled_count);
    let bar = format!("{}{}", "█".repeat(filled_count), "░".repeat(empty_count));
    let percent = (fraction * 100.0).round() as u64;
    format!("[{bar}] {percent}%")
}

/// Format MB progress as `x.x/y.y MB` (or `x.x MB` when total is unknown).
pub fn format_mb_progress(current_bytes: u64, total_bytes: u64) -> String {
    let current_mb = current_bytes as f64 / (1024.0 * 1024.0);
    if total_bytes > 0 {
        let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
        format!("{current_mb:.1}/{total_mb:.1} MB")
    } else {
        format!("{current_mb:.1} MB")
    }
}

/// Format byte progress with bar: `[██████░░░░░░] 50% (14.5/29.0 MB)`.
pub fn format_byte_progress(current_bytes: u64, total_bytes: u64, bar_length: usize) -> String {
    let bar = render_progress_bar(current_bytes, total_bytes, bar_length);
    let current_mb = current_bytes as f64 / (1024.0 * 1024.0);
    if total_bytes > 0 {
        let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
        format!("{bar} ({current_mb:.1}/{total_mb:.1} MB)")
    } else {
        format!("{current_mb:.1} MB")
    }
}

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// Format a byte count, e.g. `12.4 MB`.
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    let bytes = bytes as f64;
    let index = (bytes.ln() / 1024f64.ln()).floor().clamp(0.0, 4.0) as usize;
    let value = bytes / 1024f64.powi(index as i32);
    let formatted = if index == 0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    };
    format!("{formatted} {}", UNITS[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_zero_total_and_clamping() {
        assert_eq!(render_progress_bar(10, 0, 12), "[░░░░░░░░░░░░] 0%");
        assert_eq!(render_progress_bar(150, 100, 12), "[████████████] 100%");
        assert_eq!(render_progress_bar(50, 100, 12), "[██████░░░░░░] 50%");
        assert_eq!(render_progress_bar(1, 12, 12), "[█░░░░░░░░░░░] 8%");
    }

    #[test]
    fn mb_progress_shapes() {
        assert_eq!(
            format_mb_progress(1024 * 1024, 2 * 1024 * 1024),
            "1.0/2.0 MB"
        );
        assert_eq!(format_mb_progress(1024 * 1024, 0), "1.0 MB");
    }

    #[test]
    fn byte_progress_shapes() {
        assert_eq!(
            format_byte_progress(1024 * 1024, 2 * 1024 * 1024, 12),
            "[██████░░░░░░] 50% (1.0/2.0 MB)"
        );
        assert_eq!(format_byte_progress(1024 * 1024, 0, 12), "1.0 MB");
    }

    #[test]
    fn byte_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024u64.pow(4)), "1.00 TB");
    }
}
