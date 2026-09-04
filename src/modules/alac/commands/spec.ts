import { existsSync, unlinkSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import type { Audio, Document, Voice } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { generateSpectrogram, probeAudio } from '@/modules/alac/spectrogram.ts'
import { debug, error, infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

const AUDIO_EXTENSIONS = new Set([
  '.m4a',
  '.flac',
  '.mp3',
  '.wav',
  '.wave',
  '.aac',
  '.alac',
  '.ogg',
  '.oga',
  '.opus',
  '.aiff',
  '.aif',
  '.wma',
  '.mka',
  '.ape',
  '.wv',
  '.dsf',
  '.dff',
])

function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  if (mins >= 60) {
    const hours = Math.floor(mins / 60)
    const remainingMins = mins % 60
    return `${hours}:${String(remainingMins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`
  }
  return `${mins}:${String(secs).padStart(2, '0')}`
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
}

export function registerSpecCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx

  dp.onNewMessage(
    filters.command(['spec', 'spectogram', 'spectrogram', 'spek']),
    async (msg) => {
      using _ = infoSpan('spec_command').enter()

      const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
      if (!isAuthed && !auth.isAdmin(msg.sender.id)) {
        debug('Unauthorized user attempted /spec command', {
          user_id: msg.sender.id,
        })
        return
      }

      const reply = await msg.getReplyTo().catch(() => null)
      if (!reply?.media) {
        await msg.replyText(
          parseDynamicHtml(
            '📊 <b>Audio Spectrogram Analyzer</b><br/><br/>' +
              'Reply to any audio file, voice note, or audio document with <code>/spec</code> or <code>/spectogram</code> to generate its frequency spectrogram.<br/><br/>' +
              '<blockquote>💡 <i>Spectrograms visually expose frequency cut-offs (e.g. 16kHz for 128k MP3, 20kHz for 320k MP3), verifying genuine uncompressed lossless masters.</i></blockquote>',
          ),
        )
        return
      }

      const media = reply.media
      let ext = '.audio'
      let isAudio = false
      let fileTitle = ''

      if (media.type === 'audio') {
        isAudio = true
        ext = '.m4a'
        fileTitle = [media.performer, media.title].filter(Boolean).join(' - ')
        const fn = (media as { fileName?: string }).fileName
        if (fn && path.extname(fn)) {
          ext = path.extname(fn).toLowerCase()
        }
      } else if (media.type === 'voice') {
        isAudio = true
        ext = '.ogg'
        fileTitle = 'Voice Note'
      } else if (media.type === 'document') {
        const mime = (media as { mimeType?: string }).mimeType || ''
        const fn =
          (media as { fileName?: string }).fileName ||
          (media as { name?: string }).name ||
          ''
        const fileExt = path.extname(fn).toLowerCase()

        if (mime.startsWith('audio/') || AUDIO_EXTENSIONS.has(fileExt)) {
          isAudio = true
          ext = fileExt || '.m4a'
          fileTitle = fn.replace(/\.[^/.]+$/, '')
        }
      }

      if (
        !isAudio ||
        (media.type !== 'audio' &&
          media.type !== 'voice' &&
          media.type !== 'document')
      ) {
        await msg.replyText(
          parseDynamicHtml(
            '⚠️ <b>Unsupported Media:</b> Please reply to an audio track, voice message, or audio document.',
          ),
        )
        return
      }

      const downloadMedia: Audio | Voice | Document = media

      const statusMsg = await tg.sendText(
        msg.chat.id,
        parseDynamicHtml(
          '⏳ <i>Downloading audio for spectrogram analysis...</i>',
        ),
        { replyTo: msg.id },
      )

      const uniqueId = `${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
      const tmpAudio = path.join(os.tmpdir(), `spec_in_${uniqueId}${ext}`)
      const tmpPng = path.join(os.tmpdir(), `spec_out_${uniqueId}.png`)

      try {
        await tg.downloadToFile(tmpAudio, downloadMedia)

        await tg
          .editMessage({
            chatId: msg.chat.id,
            message: statusMsg.id,
            text: parseDynamicHtml(
              '🔬 <i>Analyzing frequencies &amp; generating spectrogram...</i>',
            ),
          })
          .catch(() => null)

        let probe = null
        try {
          probe = await probeAudio(tmpAudio)
        } catch (probeErr) {
          debug('Audio probe warning (proceeding with spectrogram)', {
            error: String(probeErr),
          })
        }

        const songTitle = probe?.title || fileTitle || 'Audio Track'
        const artist = probe?.artist
        const album = probe?.album
        const codec = (probe?.codec || ext.replace('.', '')).toUpperCase()
        const sampleRate = probe?.sampleRate || 44100
        const bitDepth = probe?.bitDepth
        const bitRate = probe?.bitRate
        const duration = probe?.duration || 0
        const channels = probe?.channels || 2

        const headerTitle = artist ? `${artist} - ${songTitle}` : songTitle
        const headerComment = `${codec}${bitDepth ? ` • ${bitDepth}-bit` : ''} • ${sampleRate.toLocaleString()} Hz${bitRate ? ` • ${Math.round(bitRate / 1000)} kbps` : ''}`

        await generateSpectrogram(tmpAudio, tmpPng, {
          title: headerTitle,
          comment: headerComment,
          duration,
        })

        let caption =
          '📊 <b>Audio Spectrogram Analysis</b><br/><br/>' +
          `• <b>Track:</b> <code>${escapeHtml(songTitle)}</code><br/>`

        if (artist) {
          caption += `• <b>Artist:</b> <code>${escapeHtml(artist)}</code><br/>`
        }
        if (album) {
          caption += `• <b>Album:</b> <code>${escapeHtml(album)}</code><br/>`
        }

        caption +=
          `• <b>Codec:</b> <code>${escapeHtml(codec)}</code><br/>` +
          `• <b>Sample Rate:</b> <code>${sampleRate.toLocaleString()} Hz (${Math.round(sampleRate / 100) / 10} kHz)</code><br/>`

        if (bitDepth) {
          caption += `• <b>Bit Depth:</b> <code>${bitDepth}-bit</code><br/>`
        }

        caption += `• <b>Channels:</b> <code>${channels === 2 ? 'Stereo (2.0)' : channels === 1 ? 'Mono' : `${channels} channels`}</code><br/>`

        if (bitRate) {
          caption += `• <b>Bitrate:</b> <code>${Math.round(bitRate / 1000)} kbps</code><br/>`
        }

        if (duration > 0) {
          caption += `• <b>Duration:</b> <code>${formatDuration(duration)}</code><br/>`
        }

        await tg.sendMedia(
          msg.chat.id,
          {
            type: 'photo',
            file: Bun.file(tmpPng),
            caption: parseDynamicHtml(caption),
          },
          { replyTo: reply.id },
        )

        await tg
          .deleteMessagesById(msg.chat.id, [statusMsg.id])
          .catch(() => null)
      } catch (err) {
        error('Failed to generate spectrogram', { error: String(err) })
        await tg
          .editMessage({
            chatId: msg.chat.id,
            message: statusMsg.id,
            text: parseDynamicHtml(
              `❌ <b>Spectrogram Generation Failed:</b> <code>${escapeHtml(String(err))}</code>`,
            ),
          })
          .catch(() => null)
      } finally {
        if (existsSync(tmpAudio)) {
          try {
            unlinkSync(tmpAudio)
          } catch {}
        }
        if (existsSync(tmpPng)) {
          try {
            unlinkSync(tmpPng)
          } catch {}
        }
      }
    },
  )
}
