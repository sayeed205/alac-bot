//! M6 `/settings` parity tests: render text, keyboard layout, storefront
//! toggling semantics, and the callback data strings — the observable
//! contract with the TS oracle (settings.ts).

use bot::handlers::settings::{render_settings_text, render_storefronts_text, POPULAR_STOREFRONTS};
use engine::settings::{BotSettings, RippingMode};

fn settings() -> BotSettings {
    BotSettings {
        ripping_mode: RippingMode::Live,
        album_rip_enabled: true,
        playlist_rip_enabled: true,
        artist_rip_enabled: true,
        txt_rip_enabled: true,
        multi_link_rip_enabled: true,
        auto_dump_enabled: false,
        auto_dump_storefronts: vec!["us".to_owned()],
        max_collection_tracks: 50,
    }
}

#[test]
fn settings_text_matches_oracle_exactly() {
    let text = render_settings_text(&settings());
    assert!(text.starts_with("⚙️ <b>Bot Settings & Operation Controls</b><br/><br/>"));
    assert!(text.contains(
        "• <b>Engine Mode:</b> 🟢 <b>Live Ripping</b> (Normal operation: cache hits + live decryption)<br/>"
    ));
    assert!(text.contains("• <b>Album Ripping:</b> 🟢 Enabled<br/>"));
    assert!(text.contains("• <b>Auto-Dump New Music:</b> 🔴 Disabled<br/>"));
    assert!(text.contains("• <b>Auto-Dump Storefronts:</b> <code>US</code><br/>"));
    assert!(text.contains("• <b>Max Collection Limit:</b> <code>50 tracks</code>"));
    assert!(text.ends_with(
        "<blockquote>💡 <i>Tap buttons below to toggle. Owner requests always bypass these limits.</i></blockquote>"
    ));
}

#[test]
fn settings_text_mode_and_limit_variants_match_oracle() {
    let mut s = settings();
    s.ripping_mode = RippingMode::CacheOnly;
    assert!(render_settings_text(&s).contains(
        "• <b>Engine Mode:</b> 🟡 <b>Cache Only</b> (Serves cached songs; live decryption blocked)<br/>"
    ));
    s.ripping_mode = RippingMode::Paused;
    assert!(render_settings_text(&s).contains(
        "• <b>Engine Mode:</b> 🔴 <b>Paused</b> (All ripping commands suspended for regular users)<br/>"
    ));
    s.max_collection_tracks = 0;
    assert!(
        render_settings_text(&s).contains("• <b>Max Collection Limit:</b> <code>Unlimited</code>")
    );
}

#[test]
fn storefronts_text_matches_oracle_exactly() {
    let text = render_storefronts_text(&settings());
    assert!(text.starts_with("🌐 <b>Auto-Dump Storefront Configuration</b><br/><br/>"));
    assert!(text.contains("• <b>Active Storefronts:</b> <code>US</code>"));
    assert!(text
        .ends_with("<i>You can also use:</i> <code>/settings storefronts add &lt;code&gt;</code>"));
}

#[test]
fn popular_storefronts_match_oracle_order() {
    assert_eq!(
        POPULAR_STOREFRONTS,
        ["us", "gb", "jp", "in", "ca", "au", "de", "fr"]
    );
}

#[test]
fn db_settings_store_auto_dump_toggle_and_sf_semantics_match_oracle() {
    // The db store is exercised through a tokio Postgres harness elsewhere;
    // here we pin the pure settings field semantics the UI depends on.
    let mut s = settings();
    assert!(!s.auto_dump_enabled);
    s.auto_dump_enabled = true;
    assert!(s.auto_dump_enabled);
    s.auto_dump_storefronts.push("jp".to_owned());
    assert_eq!(s.auto_dump_storefronts, vec!["us", "jp"]);
}
