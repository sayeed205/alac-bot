use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use ferogram::{filters, filters::Dispatcher, InputMessage, PeerRef};

use crate::{
    html::{escape, parse_dynamic_html},
    spectrogram::{self, AudioProbeResult, AUDIO_EXTENSIONS},
    BotState,
};

const USAGE: &str = "📊 <b>Audio Spectrogram Analyzer</b><br/><br/>Reply to any audio file, voice note, or audio document with <code>/spec</code> or <code>/spectogram</code> to generate its frequency spectrogram.<br/><br/><blockquote>💡 <i>Spectrograms visually expose frequency cut-offs (e.g. 16kHz for 128k MP3, 20kHz for 320k MP3), verifying genuine uncompressed lossless masters.</i></blockquote>";
const UNSUPPORTED: &str = "⚠️ <b>Unsupported Media:</b> Please reply to an audio track, voice message, or audio document.";
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct ClassifiedMedia {
    media: ferogram::tl::enums::MessageMedia,
    ext: String,
    file_title: String,
}

fn classify_media(media: &ferogram::tl::enums::MessageMedia) -> Option<ClassifiedMedia> {
    let ferogram::tl::enums::MessageMedia::Document(_) = media else {
        return None;
    };
    let document = ferogram::media::Document::from_media(media)?;
    let is_voice = document.raw.attributes.iter().any(|attribute| {
        matches!(attribute, ferogram::tl::enums::DocumentAttribute::Audio(audio) if audio.voice)
    });
    let filename = document.file_name().unwrap_or_default();
    let file_ext = Path::new(filename)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()));
    let mime = document.mime_type();
    let is_audio = is_voice
        || mime.starts_with("audio/")
        || file_ext
            .as_deref()
            .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext));
    if !is_audio {
        return None;
    }
    let (ext, file_title) = if is_voice {
        (".ogg".to_owned(), "Voice Note".to_owned())
    } else {
        let ext = file_ext.unwrap_or_else(|| ".m4a".to_owned());
        let file_title = filename
            .rfind('.')
            .map_or_else(|| filename.to_owned(), |index| filename[..index].to_owned());
        (ext, file_title)
    };
    Some(ClassifiedMedia {
        media: media.clone(),
        ext,
        file_title,
    })
}

fn temp_paths(ext: &str) -> (PathBuf, PathBuf) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let id = format!("{timestamp}_{unique}");
    (
        PathBuf::from(format!("/tmp/opencode/spec_in_{id}{ext}")),
        PathBuf::from(format!("/tmp/opencode/spec_out_{id}.png")),
    )
}

async fn edit_status(state: &BotState, peer: &PeerRef, id: i32, text: &str) {
    let _ = state
        .client
        .edit_message(
            peer.clone(),
            id,
            InputMessage::html(parse_dynamic_html(text)),
        )
        .await;
}

async fn delete_status(state: &BotState, peer: &PeerRef, id: i32) {
    if let Ok(messages) = state.client.get_messages(peer.clone(), &[id]).await {
        if let Some(message) = messages.first() {
            let _ = message.delete().await;
        }
    }
}

async fn handle(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    let chat = super::marked_chat_id(&msg);
    let authorized = state
        .auth
        .is_authorized(sender, Some(chat))
        .await
        .unwrap_or(false);
    if !authorized && !state.auth.is_admin(sender) {
        return;
    }

    let peer = super::chat_peer_ref(&msg);
    let Some(reply_id) = msg.reply_to_message_id() else {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(USAGE)))
            .await;
        return;
    };
    let Ok(replies) = state.client.get_messages(peer.clone(), &[reply_id]).await else {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(USAGE)))
            .await;
        return;
    };
    let Some(reply) = replies.first() else {
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(USAGE)))
            .await;
        return;
    };
    let Some(media) = reply.media().and_then(classify_media) else {
        let text = if reply.media().is_some() {
            UNSUPPORTED
        } else {
            USAGE
        };
        let _ = msg
            .reply(InputMessage::html(parse_dynamic_html(text)))
            .await;
        return;
    };

    let Ok(status) = state
        .client
        .send_message(
            peer.clone(),
            InputMessage::html(parse_dynamic_html(
                "⏳ <i>Downloading audio for spectrogram analysis...</i>",
            ))
            .reply_to(Some(msg.id())),
        )
        .await
    else {
        return;
    };
    let (input_path, output_path) = temp_paths(&media.ext);
    let result = async {
        state
            .client
            .download_media(
                &media.media,
                ferogram::media::MediaQuality::Original,
                &input_path,
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        edit_status(
            &state,
            &peer,
            status.id(),
            "🔬 <i>Analyzing frequencies &amp; generating spectrogram...</i>",
        )
        .await;

        let probe = spectrogram::probe_audio(&input_path)
            .await
            .unwrap_or_else(|_| AudioProbeResult {
                title: None,
                artist: None,
                album: None,
                codec: media.ext.trim_start_matches('.').to_owned(),
                sample_rate: 44_100,
                bit_depth: None,
                channels: 2,
                bit_rate: None,
                duration: 0.0,
            });
        let song_title = probe.title.clone().unwrap_or_else(|| {
            if media.file_title.is_empty() {
                "Audio Track".to_owned()
            } else {
                media.file_title.clone()
            }
        });
        let codec = probe.codec.to_uppercase();
        let header_title = probe.artist.as_ref().map_or_else(
            || song_title.clone(),
            |artist| format!("{artist} - {song_title}"),
        );
        let header_comment = format!(
            "{}{} • {} Hz{}",
            codec,
            probe
                .bit_depth
                .map_or_else(String::new, |depth| format!(" • {depth}-bit")),
            spectrogram::thousands_separator(probe.sample_rate),
            probe.bit_rate.map_or_else(String::new, |rate| format!(
                " • {} kbps",
                (rate as f64 / 1000.0).round()
            ))
        );
        spectrogram::generate_spectrogram(
            &input_path,
            &output_path,
            Some(&header_title),
            Some(&header_comment),
            probe.duration,
        )
        .await?;
        let uploaded = state
            .client
            .upload_file(&output_path)
            .await
            .map_err(|error| error.to_string())?;
        let caption = spectrogram::build_caption(&probe, &song_title, &codec);
        state
            .client
            .send_message(
                peer.clone(),
                InputMessage::html(parse_dynamic_html(&caption))
                    .copy_media(uploaded.as_photo_media())
                    .reply_to(Some(reply.id())),
            )
            .await
            .map_err(|error| error.to_string())?;
        delete_status(&state, &peer, status.id()).await;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        edit_status(
            &state,
            &peer,
            status.id(),
            &format!(
                "❌ <b>Spectrogram Generation Failed:</b> <code>{}</code>",
                escape(&error)
            ),
        )
        .await;
    }
    let _ = tokio::fs::remove_file(input_path).await;
    let _ = tokio::fs::remove_file(output_path).await;
}

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    for command in ["spec", "spectogram", "spectrogram", "spek"] {
        let state = Arc::clone(&state);
        dp.on_message(filters::command(command), move |msg| {
            let state = Arc::clone(&state);
            async move { handle(state, msg).await }
        });
    }
}
