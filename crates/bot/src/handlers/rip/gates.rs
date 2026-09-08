//! `/alac` policy gates. Strings intentionally remain literals: they are part
//! of the Telegram API of the bot (oracle `commands-rip.ts:216-307`).

use engine::{settings::BotSettings, types::ParsedTargetItem};

pub const CACHE_RESTRICTED: &str =
    "🔒 <b>Access Restricted:</b> Caching directly to dump channel is restricted to the bot owner.";
pub const FORCE_RESTRICTED: &str =
    "🔒 <b>Access Restricted:</b> Force re-rip is restricted to the bot owner.";
pub const PAUSED: &str =
    "⚠️ <b>Ripping is temporarily paused for maintenance.</b><br/>Please check back later.";
pub const ALBUM_DISABLED: &str = "⚠️ <b>Album ripping is currently disabled by admin.</b><br/>Please request individual tracks instead.";
pub const PLAYLIST_DISABLED: &str = "⚠️ <b>Playlist ripping is currently disabled by admin.</b><br/>Please request individual tracks instead.";
pub const ARTIST_DISABLED: &str = "⚠️ <b>Artist ripping is currently disabled by admin.</b><br/>Please request individual tracks or albums instead.";
pub const TXT_DISABLED: &str = "⚠️ <b>.TXT file ripping is currently disabled by admin.</b><br/>Please request individual links instead.";
pub const MULTI_DISABLED: &str = "⚠️ <b>Multi-link ripping is currently disabled by admin.</b><br/>Please request tracks or collections one at a time.";

pub fn cache_gate(is_cache_only: bool, is_admin: bool) -> Option<&'static str> {
    (is_cache_only && !is_admin).then_some(CACHE_RESTRICTED)
}

pub fn force_gate(force: bool, is_admin: bool) -> Option<&'static str> {
    (force && !is_admin).then_some(FORCE_RESTRICTED)
}

/// Returns the first feature gate which fails, preserving the TS ordering.
pub fn feature_gate(
    settings: &BotSettings,
    items: &[ParsedTargetItem],
    is_admin: bool,
    is_document: bool,
) -> Option<&'static str> {
    if is_admin {
        return None;
    }
    if !settings.can_serve_cache(false) {
        return Some(PAUSED);
    }
    if items
        .iter()
        .any(|x| x.kind == engine::types::TargetKind::Album)
        && !settings.can_rip_album(false)
    {
        return Some(ALBUM_DISABLED);
    }
    if items
        .iter()
        .any(|x| x.kind == engine::types::TargetKind::Playlist)
        && !settings.can_rip_playlist(false)
    {
        return Some(PLAYLIST_DISABLED);
    }
    if items
        .iter()
        .any(|x| x.kind == engine::types::TargetKind::Artist)
        && !settings.can_rip_artist(false)
    {
        return Some(ARTIST_DISABLED);
    }
    if is_document && !settings.can_rip_txt(false) {
        return Some(TXT_DISABLED);
    }
    if !is_document && items.len() > 1 && !settings.can_rip_multi_link(false) {
        return Some(MULTI_DISABLED);
    }
    None
}

pub fn usage(is_cache_only: bool) -> &'static str {
    if is_cache_only {
        "💾 <b>Apple Music Lossless Cacher (Admin)</b><br/><br/><blockquote><b>Usage:</b><br/>• <b>Track:</b> <code>/cache &lt;link | id&gt;</code><br/>• <b>Album:</b> <code>/cache &lt;album_link&gt;</code><br/>• <b>Playlist:</b> <code>/cache &lt;playlist_link&gt;</code><br/>• <b>Artist:</b> <code>/cache &lt;artist_link&gt;</code><br/>• <b>Batch:</b> Send multiple links or attach a <code>.txt</code> file<br/>• <b>Alias:</b> <code>/dump</code><br/>• <b>Options:</b> <code>-f</code> <i>(force re-rip even if cached)</i><br/><i>Rips and seeds lossless audio directly into dump channel and database without sending files to chat.</i></blockquote>"
    } else {
        "🎵 <b>Apple Music Lossless Ripper</b><br/><br/><blockquote><b>Supported Inputs:</b><br/>• <b>Track:</b> <code>/alac &lt;link | id&gt;</code><br/>• <b>Album:</b> <code>/alac &lt;album_link&gt;</code><br/>• <b>Playlist:</b> <code>/alac &lt;playlist_link&gt;</code><br/>• <b>Artist:</b> <code>/alac &lt;artist_link&gt;</code><br/>• <b>Batch:</b> Send multiple links or attach a <code>.txt</code> file<br/>• <b>Aliases:</b> <code>/rip</code>, <code>/batch</code>, <code>/dl</code>, <code>/download</code><br/>• <b>Cancel:</b> <code>/cancel</code> or tap the Cancel button on any active download<br/>• <b>Options:</b> <code>-f</code> <i>(force re-rip)</i></blockquote>"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_and_restrictions_are_exact() {
        assert!(
            usage(false).contains("• <b>Cancel:</b> <code>/cancel</code> or tap the Cancel button")
        );
        assert_eq!(cache_gate(true, false), Some(CACHE_RESTRICTED));
        assert_eq!(force_gate(true, false), Some(FORCE_RESTRICTED));
        assert_eq!(cache_gate(true, true), None);
    }
}
