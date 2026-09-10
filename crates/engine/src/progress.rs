//! Progress formatting helpers. Port of `src/utils/progress.ts`.

/// Render a Unicode block progress bar using `■`, `▤`, `□`,
/// e.g. `[■■■■■■□□□□□□] 50%` or `[■■■▤□□□□□□□□] 35%`.
pub fn render_progress_bar(current: u64, total: u64, length: usize) -> String {
    if total == 0 {
        return format!("[{}] 0%", "□".repeat(length));
    }
    let fraction = (current as f64 / total as f64).clamp(0.0, 1.0);
    let units = (fraction * (2.0 * length as f64)).round() as usize;
    let full_count = (units / 2).min(length);
    let half_count = if units % 2 == 1 && full_count < length {
        1
    } else {
        0
    };
    let empty_count = length.saturating_sub(full_count + half_count);
    let half_str = if half_count > 0 { "▤" } else { "" };
    let bar = format!(
        "{}{}{}",
        "■".repeat(full_count),
        half_str,
        "□".repeat(empty_count)
    );
    let percent = (fraction * 100.0).round() as u64;
    format!("[{bar}] {percent}%")
}

/// Format bytes into human-readable B, KB, MB, GB string.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2}GB", b / GB)
    } else if b >= MB {
        format!("{:.2}MB", b / MB)
    } else if b >= KB {
        format!("{:.2}KB", b / KB)
    } else {
        format!("{bytes}B")
    }
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

/// Format byte progress with bar: `[■■■■■■□□□□□□] 50% (14.5/29.0 MB)`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_examples() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(1024), "1.00KB");
        assert_eq!(format_bytes(593_000_000), "565.53MB");
        assert_eq!(format_bytes(8_799_493_473), "8.20GB");
    }
}
