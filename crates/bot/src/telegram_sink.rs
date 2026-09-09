//! Production Telegram sink used by the orchestration engine.

use std::{sync::Arc, time::Duration};

use engine::orchestrator::deps::{DumpUpload, SinkError, TelegramSink, UploadProgressCallback};
use ferogram::{InputMessage, PeerRef, TransferHandle};

/// Ferogram-backed implementation of the engine's Telegram port.
pub struct FerogramTelegramSink {
    client: Arc<ferogram::Client>,
    dump_peer: PeerRef,
    dump_peer_native_id: i64,
}

impl FerogramTelegramSink {
    pub async fn new(client: Arc<ferogram::Client>, dump_peer: PeerRef) -> Result<Self, SinkError> {
        let dump_peer = dump_peer
            .resolve(&client)
            .await
            .map_err(|error| SinkError(error.to_string()))?;
        let dump_peer_native_id = ferogram::PeerExt::bare_id(&dump_peer);
        Ok(Self {
            client,
            dump_peer: PeerRef::from(dump_peer),
            dump_peer_native_id,
        })
    }

    fn file_ids(document: &ferogram::media::Document) -> (String, String) {
        (
            format!(
                "mtproto:v1:{}:{}:{}",
                document.raw.dc_id,
                document.id(),
                document.access_hash()
            ),
            format!("mtproto:document:{}", document.id()),
        )
    }
}

impl TelegramSink for FerogramTelegramSink {
    fn send_audio_to_dump<'a>(
        &'a self,
        file_path: &'a str,
        title: &'a str,
        performer: &'a str,
        duration: i64,
        caption_html: &'a str,
        on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<DumpUpload>, SinkError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let handle = TransferHandle::new();
            let progress_task = on_upload_progress.map(|callback| {
                let callback = Arc::clone(callback);
                let handle = handle.clone();
                tokio::spawn(async move {
                    loop {
                        let progress = handle.progress();
                        callback(progress.done, progress.total);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                })
            });

            let upload_result = self.client.upload_file(file_path).handle(&handle).await;
            if let Some(task) = progress_task {
                task.abort();
            }
            let uploaded = upload_result.map_err(|error| SinkError(error.to_string()))?;

            let duration = i32::try_from(duration)
                .map_err(|error| SinkError(format!("duration out of range: {error}")))?;
            // UploadedFile intentionally exposes only the automatic media
            // builder; its InputFile is crate-private.  The returned media is
            // public, so customize the generated audio attribute in place.
            let mut media = uploaded.as_auto_media();
            if let ferogram::tl::enums::InputMedia::UploadedDocument(document) = &mut media {
                for attribute in &mut document.attributes {
                    if let ferogram::tl::enums::DocumentAttribute::Audio(audio) = attribute {
                        audio.voice = false;
                        audio.duration = duration;
                        audio.title = Some(title.to_owned());
                        audio.performer = Some(performer.to_owned());
                    }
                }
            }
            let message = self
                .client
                .send_message(
                    self.dump_peer.clone(),
                    InputMessage::html(caption_html)
                        .silent(true)
                        .copy_media(media),
                )
                .await
                .map_err(|error| SinkError(error.to_string()))?;

            tracing::info!(
                message_id = message.id(),
                dump_peer_id = self.dump_peer_native_id,
                "Audio uploaded to dump channel"
            );

            let actual_peer = message.peer_id().map(ferogram::PeerExt::bare_id);
            if actual_peer != Some(self.dump_peer_native_id) {
                return Err(SinkError(format!(
                    "dump upload landed in unexpected peer (expected {}, got {:?})",
                    self.dump_peer_native_id, actual_peer
                )));
            }

            let Some(document) = message.document() else {
                tracing::warn!(
                    message_id = message.id(),
                    "Dump upload returned no document"
                );
                return Ok(None);
            };
            let is_audio = document.raw.attributes.iter().any(|attribute| {
                matches!(attribute, ferogram::tl::enums::DocumentAttribute::Audio(_))
            });
            if !is_audio {
                return Ok(None);
            }
            let (file_id, file_unique_id) = Self::file_ids(&document);
            Ok(Some(DumpUpload {
                message_id: i64::from(message.id()),
                file_id,
                file_unique_id,
            }))
        })
    }

    fn send_dump_copy<'a>(
        &'a self,
        to_chat_id: i64,
        message_id: i64,
        reply_to: Option<i64>,
        silent: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SinkError>> + Send + 'a>>
    {
        Box::pin(async move {
            let message_id = i32::try_from(message_id)
                .map_err(|error| SinkError(format!("message_id out of range: {error}")))?;
            let reply_to = reply_to
                .map(i32::try_from)
                .transpose()
                .map_err(|error| SinkError(format!("reply_to out of range: {error}")))?;
            // TS sendDumpCopy = mtcute sendCopy with EMPTY_CAPTION: media
            // re-attached by reference, no forward attribution, empty text.
            //
            // ferogram's `copy_message` caption-override path implements
            // exactly this (fetch → re-attach → send as a fresh message),
            // but instantiating it trips a rustc layout cycle (its body
            // monomorphizes `copy_messages<T, T>`, whose body re-monomorphizes
            // `copy_message<T, T>`), so we reproduce the fetch-and-resend
            // path directly here.
            let source = self
                .client
                .get_messages(self.dump_peer.clone(), &[message_id])
                .await
                .map_err(|error| SinkError(error.to_string()))?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    SinkError("copy_message: source message not found or inaccessible".into())
                })?;

            let media: ferogram::tl::enums::InputMedia = if let Some(photo) = source.photo() {
                photo.to_input_media().into()
            } else if let Some(document) = source.document() {
                document.to_input_media().into()
            } else {
                return Err(SinkError(
                    "copy_message: source message has no copyable media".into(),
                ));
            };

            let input = InputMessage::text("")
                .reply_to(reply_to)
                .silent(silent)
                .copy_media(media);
            self.client
                .send_message(PeerRef::from(to_chat_id), input)
                .await
                .map(|_| ())
                .map_err(|error| SinkError(error.to_string()))
        })
    }

    fn delete_dump_messages<'a>(
        &'a self,
        message_ids: &'a [i64],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SinkError>> + Send + 'a>>
    {
        Box::pin(async move {
            let ids: Vec<i32> = message_ids
                .iter()
                .filter_map(|id| match i32::try_from(*id) {
                    Ok(id) => Some(id),
                    Err(error) => {
                        tracing::warn!(message_id = *id, %error, "skipping out-of-range dump message id");
                        None
                    }
                })
                .collect();
            if ids.is_empty() {
                return Ok(());
            }
            let messages = self
                .client
                .get_messages(self.dump_peer.clone(), &ids)
                .await
                .map_err(|error| SinkError(error.to_string()))?;
            for message in messages {
                if let Err(error) = message.delete_with(&self.client).await {
                    tracing::warn!(message_id = message.id(), %error, "failed to delete dump message");
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ids_match_mtproto_format() {
        let raw = ferogram::tl::types::Document {
            id: 7,
            access_hash: 11,
            file_reference: Vec::new(),
            date: 0,
            mime_type: "audio/mp4".into(),
            size: 0,
            dc_id: 2,
            attributes: Vec::new(),
            thumbs: None,
            video_thumbs: None,
        };
        let (file_id, unique_id) =
            FerogramTelegramSink::file_ids(&ferogram::media::Document::from_raw(raw));
        assert_eq!(file_id, "mtproto:v1:2:7:11");
        assert_eq!(unique_id, "mtproto:document:7");
    }
}
