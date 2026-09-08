//! `/settings` — admin settings UI (oracle:
//! `src/modules/settings/commands/settings.ts`).
//!
//! Two surfaces, identical to the oracle:
//! - `/settings` renders the inline keyboard panel; subcommands
//!   (`/settings mode|album|...|storefronts|limit`) mutate a single key and
//!   answer with the exact confirmation text.
//! - `settings:*` callbacks drive the panel (toggle/cycle/pick) and the
//!   storefront sub-menu, always re-rendering the panel after a change.
//!
//! Authorization is admin-only on both surfaces, re-checked in every
//! callback (owner id from `BotState::auth`).

use std::sync::Arc;

use ferogram::{
    filters,
    filters::Dispatcher,
    keyboard::{Button, InlineKeyboard},
    update::CallbackQuery,
    InputMessage, PeerRef,
};

use crate::{html::parse_dynamic_html, BotState};

/// Oracle POPULAR_STOREFRONTS (settings.ts:15-24).
pub const POPULAR_STOREFRONTS: [&str; 8] = ["us", "gb", "jp", "in", "ca", "au", "de", "fr"];

/// Oracle limitPresets (settings.ts:39).
const LIMIT_PRESETS: [u32; 4] = [25, 50, 100, 0];

/// Oracle modeLabels (settings.ts:29-32).
fn mode_button_label(mode: engine::settings::RippingMode) -> &'static str {
    use engine::settings::RippingMode::{CacheOnly, Live, Paused};
    match mode {
        Live => "Mode: 🟢 Live Ripping",
        CacheOnly => "Mode: 🟡 Cache Only",
        Paused => "Mode: 🔴 Fully Paused",
    }
}

fn toggle_label(label: &str, enabled: bool) -> String {
    format!("{label}: {}", if enabled { "🟢 ON" } else { "🔴 OFF" })
}

/// Oracle buildSettingsKeyboard (settings.ts:26-78).
fn settings_keyboard(settings: &engine::settings::BotSettings) -> ferogram::tl::enums::ReplyMarkup {
    let limit_buttons = LIMIT_PRESETS
        .iter()
        .map(|&preset| {
            let label = if preset == 0 {
                "Unlimited"
            } else {
                &preset.to_string()
            };
            let text = if settings.max_collection_tracks == preset {
                format!("✅ {label}")
            } else {
                label.to_owned()
            };
            Button::callback(text, format!("settings:limit:{preset}").as_bytes())
        })
        .collect::<Vec<_>>();

    let mut kb = InlineKeyboard::new()
        .row([Button::callback(
            mode_button_label(settings.ripping_mode),
            b"settings:mode",
        )])
        .row([
            Button::callback(
                toggle_label("Albums", settings.album_rip_enabled),
                b"settings:album",
            ),
            Button::callback(
                toggle_label("Playlists", settings.playlist_rip_enabled),
                b"settings:playlist",
            ),
            Button::callback(
                toggle_label("Artists", settings.artist_rip_enabled),
                b"settings:artist",
            ),
        ])
        .row([
            Button::callback(
                toggle_label(".TXT Batch", settings.txt_rip_enabled),
                b"settings:txt",
            ),
            Button::callback(
                toggle_label("Multi-Link", settings.multi_link_rip_enabled),
                b"settings:multilink",
            ),
        ])
        .row([
            Button::callback(
                toggle_label("Auto-Dump", settings.auto_dump_enabled),
                b"settings:autodump",
            ),
            Button::callback(
                format!("🌐 Storefronts ({})", settings.auto_dump_storefronts.len()),
                b"settings:sf_menu",
            ),
        ]);
    kb = kb.row(limit_buttons);
    kb.row([
        Button::callback("🔄 Refresh", b"settings:refresh"),
        Button::callback("❌ Close", b"settings:close"),
    ])
    .into_markup()
}

/// Oracle buildStorefrontsKeyboard (settings.ts:87-105).
fn storefronts_keyboard(
    settings: &engine::settings::BotSettings,
) -> ferogram::tl::enums::ReplyMarkup {
    let active: std::collections::HashSet<String> = settings
        .auto_dump_storefronts
        .iter()
        .map(|sf| sf.to_lowercase())
        .collect();
    let mut kb = InlineKeyboard::new();
    // 4 buttons per row.
    for chunk in POPULAR_STOREFRONTS.chunks(4) {
        let row = chunk
            .iter()
            .map(|&sf| {
                let active = active.contains(sf.to_lowercase().as_str());
                let label = if active {
                    format!("✅ {}", sf.to_uppercase())
                } else {
                    sf.to_uppercase()
                };
                Button::callback(label, format!("settings:sf:toggle:{sf}").as_bytes())
            })
            .collect::<Vec<_>>();
        kb = kb.row(row);
    }
    kb.row([
        Button::callback("🔙 Back to Settings", b"settings:refresh"),
        Button::callback("❌ Close", b"settings:close"),
    ])
    .into_markup()
}

/// Oracle modeDescriptions (settings.ts:112-118).
fn mode_description(mode: engine::settings::RippingMode) -> &'static str {
    use engine::settings::RippingMode::{CacheOnly, Live, Paused};
    match mode {
        Live => "🟢 <b>Live Ripping</b> (Normal operation: cache hits + live decryption)",
        CacheOnly => "🟡 <b>Cache Only</b> (Serves cached songs; live decryption blocked)",
        Paused => "🔴 <b>Paused</b> (All ripping commands suspended for regular users)",
    }
}

/// Oracle renderSettingsText (settings.ts:107-141).
pub fn render_settings_text(settings: &engine::settings::BotSettings) -> String {
    let limit_text = if settings.max_collection_tracks == 0 {
        "Unlimited".to_owned()
    } else {
        format!("{} tracks", settings.max_collection_tracks)
    };
    let sf_list = settings
        .auto_dump_storefronts
        .iter()
        .map(|sf| sf.to_uppercase())
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "⚙️ <b>Bot Settings & Operation Controls</b><br/><br/>\
• <b>Engine Mode:</b> {}<br/>\
• <b>Album Ripping:</b> {}<br/>\
• <b>Playlist Ripping:</b> {}<br/>\
• <b>Artist Ripping:</b> {}<br/>\
• <b>.TXT File Ripping:</b> {}<br/>\
• <b>Multi-Link Ripping:</b> {}<br/>\
• <b>Auto-Dump New Music:</b> {}<br/>\
• <b>Auto-Dump Storefronts:</b> <code>{sf_list}</code><br/>\
• <b>Max Collection Limit:</b> <code>{limit_text}</code><br/><br/>\
<blockquote>💡 <i>Tap buttons below to toggle. Owner requests always bypass these limits.</i></blockquote>",
        mode_description(settings.ripping_mode),
        flag(settings.album_rip_enabled),
        flag(settings.playlist_rip_enabled),
        flag(settings.artist_rip_enabled),
        flag(settings.txt_rip_enabled),
        flag(settings.multi_link_rip_enabled),
        if settings.auto_dump_enabled {
            "🟢 Enabled (Daily)"
        } else {
            "🔴 Disabled"
        },
    )
}

fn flag(enabled: bool) -> &'static str {
    if enabled {
        "🟢 Enabled"
    } else {
        "🔴 Disabled"
    }
}

/// Oracle renderStorefrontsText (settings.ts:143-156).
pub fn render_storefronts_text(settings: &engine::settings::BotSettings) -> String {
    let sf_list = settings
        .auto_dump_storefronts
        .iter()
        .map(|sf| sf.to_uppercase())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "🌐 <b>Auto-Dump Storefront Configuration</b><br/><br/>\
• <b>Active Storefronts:</b> <code>{sf_list}</code><br/><br/>\
Tap a country below to toggle it on or off for the daily new music auto-dump.<br/>\
<i>You can also use:</i> <code>/settings storefronts add &lt;code&gt;</code>"
    )
}

/// Oracle renderSettingsMessage (settings.ts:157-182): edit the panel in
/// place when a message id is given, else send fresh (replying when asked).
pub(crate) async fn render_settings_message(
    state: &BotState,
    peer: &PeerRef,
    message_id: Option<i32>,
    reply_to: Option<i32>,
) {
    let settings = state.rip_deps.settings_snapshot();
    let input = InputMessage::html(parse_dynamic_html(&render_settings_text(&settings)))
        .reply_markup(settings_keyboard(&settings));
    if let Some(message_id) = message_id {
        // NOT_MODIFIED swallow parity.
        let _ = state
            .client
            .edit_message(peer.clone(), message_id, input)
            .await;
    } else if let Some(reply_to) = reply_to {
        let _ = state
            .client
            .send_message(peer.clone(), input.reply_to(Some(reply_to)))
            .await;
    } else {
        let _ = state.client.send_message(peer.clone(), input).await;
    }
}

/// Oracle renderStorefrontsMessage (settings.ts:184-209).
pub(crate) async fn render_storefronts_message(state: &BotState, peer: &PeerRef, message_id: i32) {
    let settings = state.rip_deps.settings_snapshot();
    let input = InputMessage::html(parse_dynamic_html(&render_storefronts_text(&settings)))
        .reply_markup(storefronts_keyboard(&settings));
    // Oracle swallows all edit errors here.
    let _ = state
        .client
        .edit_message(peer.clone(), message_id, input)
        .await;
}

/// `/settings [subcommand]` — oracle registerSettingsCommands
/// (settings.ts:216-413): admin gate, subcommand mutations, else render.
pub async fn command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    if !state.auth.is_admin(sender) {
        return;
    }

    let Some(peer) = msg.peer_id() else {
        return;
    };
    let peer = PeerRef::Peer(peer.clone());

    let text = msg.text().unwrap_or_default();
    let parts: Vec<&str> = text.split_whitespace().collect();

    // Oracle: subcommands need >= 3 parts ("/settings mode live").
    if parts.len() >= 3 {
        let sub = parts[1].to_lowercase();
        let raw_value = parts[2].to_lowercase();

        if let Some(reply) = subcommand_reply(Arc::clone(&state), &sub, &raw_value, &parts).await {
            let _ = msg
                .reply(InputMessage::html(parse_dynamic_html(&reply)))
                .await;
            return;
        }
    }

    render_settings_message(&state, &peer, None, Some(msg.id())).await;
}

/// The `/settings <sub> <value>...` mutation arms. Returns the exact
/// answer text on success (Some), or None to fall through to the panel.
async fn subcommand_reply(
    state: Arc<BotState>,
    sub: &str,
    raw_value: &str,
    parts: &[&str],
) -> Option<String> {
    let settings_store = state.rip_deps.settings();

    match sub {
        "mode" => {
            let mode = match raw_value {
                "live" | "cache_only" | "paused" => raw_value,
                _ => {
                    return Some(
                        "Usage: <code>/settings mode &lt;live|cache_only|paused&gt;</code>"
                            .to_owned(),
                    )
                }
            };
            state
                .rip_deps
                .settings()
                .set_setting("ripping_mode", serde_json::json!(mode))
                .await;
            Some(format!("Engine mode set to: <b>{mode}</b>"))
        }
        "album" | "playlist" | "artist" | "txt" | "batch_txt" | "multilink" | "multi_link" => {
            let val = matches!(raw_value, "on" | "true" | "1");
            let (key, label) = match sub {
                "album" => ("album_rip_enabled", "Album ripping"),
                "playlist" => ("playlist_rip_enabled", "Playlist ripping"),
                "artist" => ("artist_rip_enabled", "Artist ripping"),
                "txt" | "batch_txt" => ("txt_rip_enabled", ".TXT batch ripping"),
                _ => ("multi_link_rip_enabled", "Multi-link ripping"),
            };
            settings_store
                .set_setting(key, serde_json::json!(val))
                .await;
            Some(format!(
                "{label} set to: <b>{}</b>",
                if val { "ON" } else { "OFF" }
            ))
        }
        "autodump" | "auto_dump" => {
            let val = matches!(raw_value, "on" | "true" | "1");
            settings_store
                .set_setting("auto_dump_enabled", serde_json::json!(val))
                .await;
            Some(format!(
                "Auto-dump new music set to: <b>{}</b>",
                if val { "ON" } else { "OFF" }
            ))
        }
        "storefronts" | "storefront" | "sf" => {
            let action = raw_value;
            let target_sf = parts.get(3).map(|s| s.to_lowercase());
            match (action, target_sf) {
                ("add", Some(target_sf)) => {
                    let list = settings_store.add_auto_dump_storefront(&target_sf).await;
                    Some(format!(
                        "Added <b>{}</b>. Storefronts: <code>{}</code>",
                        target_sf.to_uppercase(),
                        upper_join(&list)
                    ))
                }
                ("remove" | "rm" | "del", Some(target_sf)) => {
                    let list = settings_store.remove_auto_dump_storefront(&target_sf).await;
                    Some(format!(
                        "Removed <b>{}</b>. Storefronts: <code>{}</code>",
                        target_sf.to_uppercase(),
                        upper_join(&list)
                    ))
                }
                ("set", _) => {
                    let targets = parts[3..]
                        .iter()
                        .flat_map(|part| part.split(','))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    let list = settings_store.set_auto_dump_storefronts(&targets).await;
                    Some(format!(
                        "Storefronts set to: <code>{}</code>",
                        upper_join(&list)
                    ))
                }
                _ => Some(
                    "Usage: <code>/settings storefronts &lt;add|rm|set&gt; &lt;code&gt;</code>"
                        .to_owned(),
                ),
            }
        }
        "limit" => {
            // Oracle: parse failure also answers with the usage string.
            let num: i64 = match raw_value.parse() {
                Ok(num) => num,
                Err(_) => {
                    return Some(
                        "Usage: <code>/settings limit &lt;number (0 for unlimited)&gt;</code>"
                            .to_owned(),
                    )
                }
            };
            if num >= 0 {
                let updated = settings_store.set_max_collection_tracks(num).await;
                Some(format!(
                    "Max collection limit set to: <b>{}</b>",
                    if updated == 0 {
                        "Unlimited".to_owned()
                    } else {
                        updated.to_string()
                    }
                ))
            } else {
                Some(
                    "Usage: <code>/settings limit &lt;number (0 for unlimited)&gt;</code>"
                        .to_owned(),
                )
            }
        }
        _ => None,
    }
}

fn upper_join(list: &[String]) -> String {
    list.iter()
        .map(|sf| sf.to_uppercase())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `settings:*` callbacks — oracle settings.ts:416-547.
pub async fn callback(state: Arc<BotState>, query: CallbackQuery) {
    let Some(data) = query.data().map(str::to_owned) else {
        return;
    };

    // Admin re-check on every callback (owner only).
    if !state.auth.is_admin(query.user_id) {
        let _ = query
            .answer()
            .alert("Unauthorized. Owner only.")
            .send(&state.client)
            .await;
        return;
    }

    let peer = query.chat_peer.clone().map(PeerRef::Peer);
    let message_id = query.message_id;

    match data.as_str() {
        "settings:close" => {
            let _ = query.answer().send(&state.client).await;
            if let (Some(peer), Some(id)) = (peer, message_id) {
                delete_panel_message(state, peer, id).await;
            }
        }
        "settings:refresh" => {
            let _ = query
                .answer()
                .text("Settings refreshed")
                .send(&state.client)
                .await;
            if let (Some(peer), Some(id)) = (peer, message_id) {
                render_settings_message(&state, &peer, Some(id), None).await;
            }
        }
        "settings:sf_menu" => {
            let _ = query.answer().send(&state.client).await;
            if let (Some(peer), Some(id)) = (peer, message_id) {
                render_storefronts_message(&state, &peer, id).await;
            }
        }
        "settings:mode" => {
            let new_mode = state.rip_deps.settings().cycle_ripping_mode().await;
            let label = match new_mode {
                engine::settings::RippingMode::Live => "Mode: Live Ripping",
                engine::settings::RippingMode::CacheOnly => "Mode: Cache Only",
                engine::settings::RippingMode::Paused => "Mode: Fully Paused",
            };
            let _ = query.answer().text(label).send(&state.client).await;
            if let (Some(peer), Some(id)) = (peer, message_id) {
                render_settings_message(&state, &peer, Some(id), None).await;
            }
        }
        "settings:album" | "settings:playlist" | "settings:artist" | "settings:txt"
        | "settings:multilink" | "settings:autodump" => {
            let settings_store = state.rip_deps.settings();
            let (enabled, label) = match data.as_str() {
                "settings:album" => (settings_store.toggle_album().await, "Album ripping"),
                "settings:playlist" => (settings_store.toggle_playlist().await, "Playlist ripping"),
                "settings:artist" => (settings_store.toggle_artist().await, "Artist ripping"),
                "settings:txt" => (settings_store.toggle_txt().await, ".TXT batch ripping"),
                "settings:multilink" => (
                    settings_store.toggle_multi_link_rip().await,
                    "Multi-link ripping",
                ),
                _ => (settings_store.toggle_auto_dump().await, "Auto-dump"),
            };
            let _ = query
                .answer()
                .text(format!("{label}: {}", if enabled { "ON" } else { "OFF" }))
                .send(&state.client)
                .await;
            if let (Some(peer), Some(id)) = (peer, message_id) {
                render_settings_message(&state, &peer, Some(id), None).await;
            }
        }
        _ => {
            // settings:limit:<n> and settings:sf:toggle:<sf>
            if let Some(limit_str) = data.strip_prefix("settings:limit:") {
                let Ok(limit) = limit_str.parse::<i64>() else {
                    return;
                };
                if limit >= 0 {
                    state
                        .rip_deps
                        .settings()
                        .set_max_collection_tracks(limit)
                        .await;
                    let _ = query
                        .answer()
                        .text(format!(
                            "Collection limit: {}",
                            if limit == 0 {
                                "Unlimited".to_owned()
                            } else {
                                format!("{limit} tracks")
                            }
                        ))
                        .send(&state.client)
                        .await;
                    if let (Some(peer), Some(id)) = (peer, message_id) {
                        render_settings_message(&state, &peer, Some(id), None).await;
                    }
                }
            } else if let Some(sf) = data.strip_prefix("settings:sf:toggle:") {
                let sf = sf.to_lowercase();
                let current = state.rip_deps.settings_snapshot().auto_dump_storefronts;
                if current.iter().any(|value| value.eq_ignore_ascii_case(&sf)) {
                    state
                        .rip_deps
                        .settings()
                        .remove_auto_dump_storefront(&sf)
                        .await;
                    let _ = query
                        .answer()
                        .text(format!("Removed {}", sf.to_uppercase()))
                        .send(&state.client)
                        .await;
                } else {
                    state
                        .rip_deps
                        .settings()
                        .add_auto_dump_storefront(&sf)
                        .await;
                    let _ = query
                        .answer()
                        .text(format!("Added {}", sf.to_uppercase()))
                        .send(&state.client)
                        .await;
                }
                if let (Some(peer), Some(id)) = (peer, message_id) {
                    render_storefronts_message(&state, &peer, id).await;
                }
            }
        }
    }
}

/// Channel-aware panel deletion, same pattern as the auth-list close button.
async fn delete_panel_message(state: Arc<BotState>, peer: PeerRef, id: i32) {
    if let Ok(messages) = state.client.get_messages(peer, &[id]).await {
        if let Some(message) = messages.first() {
            let _ = message.delete().await;
        }
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("settings"), move |msg| {
        let state = Arc::clone(&state);
        async move {
            command(state, msg).await;
        }
    });
}
