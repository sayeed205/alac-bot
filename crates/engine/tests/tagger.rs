//! Filename policy tests.

use engine::{
    tagger::{
        bound_filename_component, bound_filename_with_suffix, build_track_filename,
        build_track_filename_with_codec, sanitize_filename,
    },
    types::TrackMeta,
};

fn meta() -> TrackMeta {
    TrackMeta {
        id: "1".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        album_artist: "Artist".into(),
        genre: None,
        release_date: String::new(),
        composer: None,
        track_number: Some(3),
        track_count: Some(12),
        disc_number: Some(1),
        disc_count: Some(2),
        duration_secs: 100,
        explicit: false,
        content_advisory: None,
        artwork_url: String::new(),
        album_id: None,
        artist_id: None,
        isrc: None,
        record_label: None,
        copyright: None,
        upc: None,
        is_streamable: None,
    }
}

#[test]
fn sanitize_replaces_and_defaults() {
    assert_eq!(
        sanitize_filename("a<b>c:d\"e/f\\g|h?i*j"),
        "a_b_c_d_e_f_g_h_i_j"
    );
    assert_eq!(sanitize_filename("   "), "track");
    assert_eq!(sanitize_filename("  ok  "), "ok");
}

#[test]
fn filename_explicit_and_number_padding() {
    let mut value = meta();
    value.explicit = true;
    assert_eq!(
        build_track_filename(&value),
        "03. Song - Artist [E] [ALAC].m4a"
    );
    value.explicit = false;
    value.track_number = Some(0);
    assert_eq!(build_track_filename(&value), "01. Song - Artist [ALAC].m4a");
    value.track_number = Some(123);
    assert_eq!(
        build_track_filename(&value),
        "123. Song - Artist [ALAC].m4a"
    );
}

#[test]
fn filename_labels_canonical_aac_primary_codec() {
    assert_eq!(
        build_track_filename_with_codec(&meta(), "aac"),
        "03. Song - Artist [AAC].m4a"
    );
}

#[test]
fn bounds_filename_at_utf8_boundary_without_losing_suffix() {
    let name = format!("{} [AAC].m4a", "é".repeat(200));
    let suffix = " [AAC].m4a";
    let bounded = bound_filename_with_suffix(&name, suffix, 255);

    assert!(bounded.len() <= 255);
    assert!(bounded.ends_with(suffix));
    assert_eq!(
        bounded,
        format!("{}{}", bound_filename_component(&name[..400], 244), suffix)
    );
}
