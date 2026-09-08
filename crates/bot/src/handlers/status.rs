use std::{sync::Arc, time::Duration};

use ferogram::{
    filters::{self, Dispatcher},
    ErrorKind, InputMessage, InvocationErrorExt, PeerRef,
};

use crate::{
    dashboard::{DashboardFuture, DashboardSink, DashboardSnapshot, EditError},
    dashboard_manager, BotState,
};

struct TelegramSink {
    client: ferogram::Client,
    peer: PeerRef,
}

fn edit_error(error: ferogram::InvocationError) -> EditError {
    match error.kind() {
        ErrorKind::FloodWait(seconds) => EditError::FloodWait(Duration::from_secs(seconds)),
        _ => EditError::Other(error.to_string()),
    }
}
impl DashboardSink for TelegramSink {
    fn send<'a>(
        &'a self,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> DashboardFuture<'a, i32> {
        Box::pin(async move {
            let mut input = InputMessage::html(text);
            if let Some(k) = keyboard {
                input = input.reply_markup(k);
            }
            self.client
                .send_message(self.peer.clone(), input)
                .await
                .map(|m| m.id())
                .map_err(edit_error)
        })
    }
    fn edit<'a>(
        &'a self,
        id: i32,
        text: &'a str,
        keyboard: Option<ferogram::tl::enums::ReplyMarkup>,
    ) -> DashboardFuture<'a, ()> {
        Box::pin(async move {
            let mut input = InputMessage::html(text);
            if let Some(k) = keyboard {
                input = input.reply_markup(k);
            }
            self.client
                .edit_message(self.peer.clone(), id, input)
                .await
                .map_err(edit_error)
        })
    }
    fn delete<'a>(&'a self, id: i32) -> DashboardFuture<'a, ()> {
        Box::pin(async move {
            let messages = self
                .client
                .get_messages(self.peer.clone(), &[id])
                .await
                .map_err(edit_error)?;
            if let Some(message) = messages.first() {
                message.delete().await.map_err(edit_error)?;
            }
            Ok(())
        })
    }
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    dp.on_message(filters::command("status"), move |msg| {
        let state = Arc::clone(&state);
        async move {
            let user = msg.sender_user_id().unwrap_or_default();
            if !state
                .auth
                .is_authorized(user, Some(msg.chat_id()))
                .await
                .unwrap_or(false)
            {
                return;
            }
            let sink: Arc<dyn DashboardSink> = Arc::new(TelegramSink {
                client: state.client.clone(),
                peer: PeerRef::from(msg.chat_id()),
            });
            let _ = dashboard_manager()
                .open(
                    msg.chat_id(),
                    user,
                    sink,
                    DashboardSnapshot {
                        ripping_mode: "sequential".into(),
                        mirror_health: None,
                        jobs: Vec::new(),
                    },
                )
                .await;
        }
    });
}
