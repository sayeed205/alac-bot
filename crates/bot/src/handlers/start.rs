use std::sync::Arc;

use ferogram::filters::{self, Dispatcher};

use crate::BotState;

pub fn register(dp: &mut Dispatcher, _state: Arc<BotState>) {
    dp.on_message(filters::command("start"), |msg| async move {
        if let Err(error) = msg.reply("Hello, world!").await {
            tracing::warn!(%error, "start reply failed");
        }
    });
}
