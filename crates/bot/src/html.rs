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
    content.to_owned()
}
