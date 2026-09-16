//! Typed provenance for an acquired stream source.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceId {
    PrimaryMirror,
    WrapperLite { url: String },
    WrapperCandidate { endpoint: String },
    QobuzBackend { url: String },
    QobuzNative,
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrimaryMirror => formatter.write_str("primary mirror"),
            Self::WrapperLite { url } => write!(formatter, "wrapper-lite ({url})"),
            Self::WrapperCandidate { endpoint } => {
                write!(formatter, "wrapper candidate ({endpoint})")
            }
            Self::QobuzBackend { url } => write!(formatter, "qobuz backend ({url})"),
            Self::QobuzNative => formatter.write_str("qobuz native ripper"),
        }
    }
}
