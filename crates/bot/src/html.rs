/// Escape dynamic text for Telegram HTML.
pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Prepare the dynamic HTML used by the TypeScript bot for ferogram's parser.
/// Ferogram's parser understands all the tags the TS oracle emits, including
/// `<br>`, `<blockquote>`, and `<blockquote expandable>`, natively, so the
/// content passes through unchanged. This helper is kept as the single place
/// where TS-vs-ferogram HTML dialect differences are reconciled.
pub fn parse_dynamic_html(content: &str) -> String {
    // Presentation policy: decorative emoji are not used to carry meaning.
    // Keep the semantic ASCII markers used by the new renderer (✓, !, ×, …)
    // and remove legacy pictographs at the final Telegram seam so any
    // unmigrated administrative handler still follows the UX rule.
    content
        .chars()
        .filter(|ch| {
            !matches!(
                *ch,
                '✅' | '❌'
                    | '⚠'
                    | '🎵'
                    | '🎧'
                    | '📀'
                    | '📁'
                    | '📊'
                    | '🔄'
                    | '⏳'
                    | '🗑'
                    | '🔍'
                    | '🎲'
                    | '💾'
                    | '📥'
                    | '📤'
                    | '🔗'
                    | '🛠'
                    | '⚙'
                    | 'ℹ'
                    | '🚫'
                    | '👋'
                    | '🔥'
                    | '💿'
                    | '📋'
                    | '🧹'
                    | '🗒'
                    | '🟢'
                    | '🔴'
                    | '🟡'
                    | '📩'
                    | '🏓'
                    | '⚡'
                    | '🌐'
                    | '💡'
                    | '🚨'
                    | '✂'
                    | '🏷'
                    | '✍'
                    | '🛑'
                    | '📡'
                    | '🔙'
                    | '⬅'
                    | '👉'
                    | '🚀'
                    | '💽'
                    | '🆔'
                    | '🖥'
                    | '🔒'
                    | '⛔'
                    | '\u{fe0f}'
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_html_seam_removes_legacy_decoration_but_keeps_semantic_markers() {
        assert_eq!(parse_dynamic_html("⚠️ <b>Paused</b> 🎵"), " <b>Paused</b> ");
        assert_eq!(parse_dynamic_html("✓ <b>Complete</b>"), "✓ <b>Complete</b>");
    }
}
