//! URL parsing and identification for Qobuz entities.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QobuzKind {
    Track,
    Album,
    Artist,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QobuzEntity {
    pub kind: QobuzKind,
    pub id: String,
}

static QOBUZ_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)https?://(?:open|play|www)\.qobuz\.com/(?:[a-z]{2}-[a-z]{2}/)?(track|album|artist|interpreter|interprete|playlist)/(?:[^/]+/)?([a-zA-Z0-9_-]+)"#)
        .expect("valid regex")
});

static QOBUZ_URI_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)^qobuz:(track|album|artist|interpreter|interprete|playlist):([a-zA-Z0-9_-]+)$"#)
        .expect("valid regex")
});

pub fn parse_qobuz_url(input: &str) -> Option<QobuzEntity> {
    let input = input.trim();

    if let Some(caps) = QOBUZ_URL_REGEX.captures(input) {
        let kind_str = caps.get(1)?.as_str().to_ascii_lowercase();
        let id = caps.get(2)?.as_str().to_owned();
        let kind = match kind_str.as_str() {
            "track" => QobuzKind::Track,
            "album" => QobuzKind::Album,
            "artist" | "interpreter" | "interprete" => QobuzKind::Artist,
            "playlist" => QobuzKind::Playlist,
            _ => return None,
        };
        return Some(QobuzEntity { kind, id });
    }

    if let Some(caps) = QOBUZ_URI_REGEX.captures(input) {
        let kind_str = caps.get(1)?.as_str().to_ascii_lowercase();
        let id = caps.get(2)?.as_str().to_owned();
        let kind = match kind_str.as_str() {
            "track" => QobuzKind::Track,
            "album" => QobuzKind::Album,
            "artist" | "interpreter" | "interprete" => QobuzKind::Artist,
            "playlist" => QobuzKind::Playlist,
            _ => return None,
        };
        return Some(QobuzEntity { kind, id });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_and_play_links() {
        assert_eq!(
            parse_qobuz_url("https://play.qobuz.com/track/46487920"),
            Some(QobuzEntity {
                kind: QobuzKind::Track,
                id: "46487920".to_owned()
            })
        );
        assert_eq!(
            parse_qobuz_url("https://open.qobuz.com/album/0060253782782"),
            Some(QobuzEntity {
                kind: QobuzKind::Album,
                id: "0060253782782".to_owned()
            })
        );
        assert_eq!(
            parse_qobuz_url("https://www.qobuz.com/us-en/album/bad-blood-bastille/0060253782782"),
            Some(QobuzEntity {
                kind: QobuzKind::Album,
                id: "0060253782782".to_owned()
            })
        );
        assert_eq!(
            parse_qobuz_url("qobuz:track:46487920"),
            Some(QobuzEntity {
                kind: QobuzKind::Track,
                id: "46487920".to_owned()
            })
        );
        assert_eq!(
            parse_qobuz_url("https://www.qobuz.com/us-en/interpreter/billie-eilish/2867335"),
            Some(QobuzEntity {
                kind: QobuzKind::Artist,
                id: "2867335".to_owned()
            })
        );
        assert_eq!(
            parse_qobuz_url("https://play.qobuz.com/artist/2867335"),
            Some(QobuzEntity {
                kind: QobuzKind::Artist,
                id: "2867335".to_owned()
            })
        );
    }
}
