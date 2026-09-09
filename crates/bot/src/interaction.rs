//! Typed Telegram callback protocol.
//!
//! All callback payloads cross this seam once. Feature modules may still own
//! their domain transitions, but malformed data can never fall through to an
//! unrelated handler.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramAction {
    Dashboard {
        action: DashboardAction,
        page: usize,
    },
    Cancel {
        job_id: String,
    },
    Settings {
        payload: Vec<String>,
    },
    Report {
        payload: Vec<String>,
    },
    Discovery {
        payload: Vec<String>,
    },
    DeliverCached {
        track_id: String,
    },
    Rip {
        track_id: String,
    },
    SearchClose,
    AuthPage {
        page: usize,
    },
    AuthClose,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardAction {
    Previous,
    Next,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Empty,
    Unknown,
    Malformed,
    Oversized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackAck {
    Toast(String),
    Alert(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageEffect {
    Edit(String),
    Delete,
    None,
}

/// One callback query produces exactly one acknowledgement and at most one
/// message effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackOutcome {
    pub acknowledgement: CallbackAck,
    pub effect: MessageEffect,
}

const MAX_PAYLOAD: usize = 256;

impl TelegramAction {
    pub fn decode(data: &str) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Empty);
        }
        if data.len() > MAX_PAYLOAD {
            return Err(DecodeError::Oversized);
        }
        if data == "search_close" {
            return Ok(Self::SearchClose);
        }
        if data == "authclose" {
            return Ok(Self::AuthClose);
        }
        if data == "noop" {
            return Ok(Self::Noop);
        }
        let mut parts = data.split(':');
        let prefix = parts.next().ok_or(DecodeError::Malformed)?;
        let rest = parts.collect::<Vec<_>>();
        match prefix {
            "cancel" => one(rest).map(|job_id| Self::Cancel { job_id }),
            "dl" => one(rest).map(|track_id| Self::DeliverCached { track_id }),
            "rip" => one(rest).map(|track_id| Self::Rip { track_id }),
            "authpage" => one(rest)
                .and_then(|value| parse_page(&value))
                .map(|page| Self::AuthPage { page }),
            "dashboard" => {
                if rest.len() != 2 {
                    return Err(DecodeError::Malformed);
                }
                let action = match rest[0] {
                    "prev" => DashboardAction::Previous,
                    "next" => DashboardAction::Next,
                    "refresh" => DashboardAction::Refresh,
                    _ => return Err(DecodeError::Unknown),
                };
                parse_page(rest[1]).map(|page| Self::Dashboard { action, page })
            }
            "settings" => non_empty(rest).map(|payload| Self::Settings { payload }),
            "report" => non_empty(rest).map(|payload| Self::Report { payload }),
            "random" => non_empty(rest).map(|payload| Self::Discovery { payload }),
            _ => Err(DecodeError::Unknown),
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Dashboard { action, page } => format!(
                "dashboard:{}:{page}",
                match action {
                    DashboardAction::Previous => "prev",
                    DashboardAction::Next => "next",
                    DashboardAction::Refresh => "refresh",
                }
            ),
            Self::Cancel { job_id } => format!("cancel:{job_id}"),
            Self::Settings { payload } => join("settings", payload),
            Self::Report { payload } => join("report", payload),
            Self::Discovery { payload } => join("random", payload),
            Self::DeliverCached { track_id } => format!("dl:{track_id}"),
            Self::Rip { track_id } => format!("rip:{track_id}"),
            Self::SearchClose => "search_close".to_owned(),
            Self::AuthPage { page } => format!("authpage:{page}"),
            Self::AuthClose => "authclose".to_owned(),
            Self::Noop => "noop".to_owned(),
        }
    }
}

fn one(parts: Vec<&str>) -> Result<String, DecodeError> {
    if parts.len() == 1 && !parts[0].is_empty() {
        Ok(parts[0].to_owned())
    } else {
        Err(DecodeError::Malformed)
    }
}

fn non_empty(parts: Vec<&str>) -> Result<Vec<String>, DecodeError> {
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        Err(DecodeError::Malformed)
    } else {
        Ok(parts.into_iter().map(str::to_owned).collect())
    }
}

fn parse_page(value: &str) -> Result<usize, DecodeError> {
    value
        .parse::<usize>()
        .map_err(|_| DecodeError::Malformed)
        .and_then(|page| (page > 0).then_some(page).ok_or(DecodeError::Malformed))
}

fn join(prefix: &str, payload: &[String]) -> String {
    std::iter::once(prefix.to_owned())
        .chain(payload.iter().cloned())
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_actions_round_trip() {
        let actions = [
            TelegramAction::Dashboard {
                action: DashboardAction::Next,
                page: 2,
            },
            TelegramAction::Cancel {
                job_id: "job-1".into(),
            },
            TelegramAction::Settings {
                payload: vec!["sf".into(), "toggle".into(), "in".into()],
            },
            TelegramAction::Report {
                payload: vec!["act".into(), "dismiss".into(), "7".into()],
            },
            TelegramAction::Discovery {
                payload: vec!["reroll".into(), "charts".into()],
            },
            TelegramAction::DeliverCached {
                track_id: "1".into(),
            },
            TelegramAction::Rip {
                track_id: "2".into(),
            },
            TelegramAction::SearchClose,
            TelegramAction::AuthPage { page: 3 },
            TelegramAction::AuthClose,
            TelegramAction::Noop,
        ];
        for action in actions {
            assert_eq!(TelegramAction::decode(&action.encode()), Ok(action));
        }
    }

    #[test]
    fn invalid_callbacks_are_rejected() {
        assert_eq!(
            TelegramAction::decode("unknown:thing"),
            Err(DecodeError::Unknown)
        );
        assert_eq!(
            TelegramAction::decode("dashboard:next:0"),
            Err(DecodeError::Malformed)
        );
        assert_eq!(
            TelegramAction::decode("cancel:"),
            Err(DecodeError::Malformed)
        );
    }
}
