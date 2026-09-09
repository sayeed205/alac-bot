use std::sync::Arc;

use ferogram::{filters, filters::Dispatcher, InputMessage};

use crate::{html::parse_dynamic_html, BotState};

const HELP: &str =
    "📖 <b>ALAC Bot Usage Guide</b><br/><br/>\
<blockquote><b>General Commands:</b><br/>\
• <code>/alac &lt;link | id&gt;</code> — Rip track, album, or playlist in lossless ALAC<br/>\
• <code>/batch &lt;links...&gt;</code> — Download multiple links or attach a <code>.txt</code> file<br/>\
• <code>/cancel</code> — Cancel an active download task<br/>\
• <code>/spec</code> — Reply to audio file to generate frequency spectrogram<br/>\
• <code>/report</code> — Reply to song to report corruption or issues to admin<br/>\
• <code>/search &lt;query&gt;</code> — Search cached tracks for instant download<br/>\
• <code>/info &lt;link | id&gt;</code> — Inspect track details & check cache status<br/>\
• <code>/queue</code> — Check active & pending rip jobs<br/>\
• <code>/status</code> — Check live download status & queue dashboard<br/>\
• <code>/ping</code> — Test bot latency & system health<br/>\
• <code>/help</code> — Show this usage guide</blockquote><br/>\
<blockquote expandable>💡 <b>Ripping Tips:</b><br/>\
• <b>Aliases:</b> <code>/rip</code>, <code>/batch</code>, <code>/dl</code>, <code>/download</code><br/>\
• <b>Cancel Download:</b> Tap <code>❌ Cancel Download</code> button on the progress card or use <code>/cancel</code>.<br/>\
• <b>Spectrogram:</b> Reply to any audio file with <code>/spec</code> or <code>/spectogram</code> to visually verify uncompressed lossless quality.<br/>\
• <b>Reporting Issues:</b> Reply to any corrupt or cut-off track with <code>/report</code> to notify the admin for a re-rip.<br/>\
• <b>Group Chats:</b> Audio files are delivered to your private DM to keep the chat clean!<br/>\
• <b>Playlists:</b> Paste any Apple Music playlist link to download all songs.<br/>\
• <b>Batch Files:</b> Send or reply to a <code>.txt</code> file containing links with <code>/alac</code>.<br/>\
• Synchronized lyrics (Enhanced LRC) and cover artwork are automatically embedded.</blockquote>";

const ADMIN: &str =
    "<br/><br/><blockquote><b>Admin Commands:</b><br/>\
• <code>/settings</code> — Bot operational settings & ripping toggles<br/>\
• <code>/dumpnew &lt;days&gt;</code> — Auto-dump new releases from Apple Music (alias: <code>/autodump</code>)<br/>\
• <code>/cache &lt;link&gt;</code> — Pre-cache/seed tracks directly into dump channel without sending audio (alias: <code>/dump</code>)<br/>\
• <code>/random</code> — Interactive random album discovery & dump<br/>\
• <code>/alac &lt;link&gt; -f</code> or <code>/rerip</code> — Force re-rip, bypassing cache<br/>\
• <code>/delete &lt;track_id&gt;</code> — Delete track from DB & dump channel<br/>\
• <code>/auth &lt;user_id | reply&gt;</code> — Whitelist user<br/>\
• <code>/revoke &lt;user_id | reply&gt;</code> — Revoke user access<br/>\
• <code>/authlist</code> — List authorized users<br/>\
• <code>/stats</code> — View bot download & cache statistics<br/>\
 • <code>/clean</code> — Remove leftover temporary files from download storage</blockquote>";

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("help"), move |msg| {
        let state = Arc::clone(&state);
        async move {
            let sender = match msg.sender_user_id() {
                Some(id) => id,
                None => return,
            };
            // Parity: the TS oracle passes msg.chat.id (the Bot-API marked
            // id) as the chat scope for group authorization checks.
            let chat = msg.peer_id().map(super::marked_peer_id);
            match state.auth.is_authorized(sender, chat).await {
                Ok(true) => {
                    let text = if state.auth.is_admin(sender) {
                        format!("{HELP}{ADMIN}")
                    } else {
                        HELP.to_owned()
                    };
                    if let Err(error) = msg
                        .reply(InputMessage::html(parse_dynamic_html(&text)))
                        .await
                    {
                        tracing::warn!(%error, "help reply failed");
                    }
                }
                Ok(false) => {}
                Err(error) => tracing::error!(%error, "authorization lookup failed"),
            }
        }
    });
}
