//! `/dumpnew` `/autodump` — new-release archiver + 24h scheduler (M8).
//! Implemented in the autodump lane.
use std::sync::Arc;

use ferogram::filters::Dispatcher;

use crate::BotState;

pub fn register(_dp: &mut Dispatcher, _state: Arc<BotState>) {}
