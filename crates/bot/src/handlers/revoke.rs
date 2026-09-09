use std::sync::Arc;

use ferogram::{filters, filters::Dispatcher, InputMessage};

use super::auth::resolve_target;
use crate::{
    html::{escape, parse_dynamic_html},
    BotState,
};

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    for command in ["revoke", "unauth"] {
        let state = Arc::clone(&state);
        dp.on_message(filters::command(command), move |msg| {
            let state = Arc::clone(&state);
            async move {
                let sender = match msg.sender_user_id() { Some(id) => id, None => return };
                if !state.auth.is_admin(sender) { return; }
                let Some((id, name, _is_user)) = resolve_target(&msg, &state).await else {
                    let usage = "<b>Revocation usage</b><br/><br/><blockquote>• Reply to a message with <code>/revoke</code><br/>• <code>/revoke &lt;id | @username&gt;</code><br/>• Send <code>/revoke</code> inside a group to revoke the group</blockquote>";
                    let _ = msg.reply(InputMessage::html(parse_dynamic_html(usage))).await;
                    return;
                };
                match state.auth.revoke(id).await {
                    Ok(true) => {
                        let link = super::peer_link(&name, id);
                        let text = format!("✓ <b>Revoked access for:</b> <a href=\"{link}\">{}</a> (<code>{id}</code>)", escape(&name));
                        let _ = msg.reply(InputMessage::html(parse_dynamic_html(&text))).await;
                    }
                    Ok(false) => { let _ = msg.reply(InputMessage::html(parse_dynamic_html(&format!("! <b>Not found:</b> ID <code>{id}</code> was not in the authorized list.")))).await; }
                    Err(error) => tracing::error!(%error, "revocation failed"),
                }
            }
        });
    }
}
