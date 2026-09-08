//! `/search` — local + live catalog search (M8). Implemented in the search lane.
use std::sync::Arc;

use ferogram::{filters::Dispatcher, update::CallbackQuery};

use crate::BotState;

pub fn register(_dp: &mut Dispatcher, _state: Arc<BotState>) {}
pub async fn callback(_state: Arc<BotState>, _query: CallbackQuery) {}
