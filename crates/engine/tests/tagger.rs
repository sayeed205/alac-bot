//! Filename policy tests.

use engine::{
    tagger::{build_track_filename, sanitize_filename},
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
