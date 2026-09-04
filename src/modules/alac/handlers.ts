import { existsSync, unlinkSync } from 'node:fs'

import { html, type TelegramClient } from '@mtcute/bun'
import { type Dispatcher, filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { fetchAlbumTracks } from '@/modules/alac/itunes.ts'
import { parseAlacInput } from '@/modules/alac/parser.ts'
import {
  ripQueue as defaultQueue,
  type IRipQueue,
} from '@/modules/alac/queue.ts'
import {
  defaultRipper,
  type ITrackRipper,
  type TrackRipResult,
} from '@/modules/alac/ripper.ts'
import {
  alacService as defaultService,
  type IAlacService,
} from '@/modules/alac/service.ts'
import { authService, type IAuthService } from '@/modules/auth/service.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

function parseDynamicHtml(content: string) {
  return html([content] as unknown as TemplateStringsArray)
}

export function registerAlacHandlers(
  dp: Dispatcher<TelegramClient>,
  tg: TelegramClient,
  service: IAlacService = defaultService,
  ripper: ITrackRipper = defaultRipper,
  queue: IRipQueue = defaultQueue,
  auth: IAuthService = authService,
) {
  dp.onNewMessage(filters.command(['alac', 'rerip']), async (msg) => {
    using _cmdSpan = infoSpan('alac_cmd', {
      user_id: msg.sender.id,
      chat_id: msg.chat.id,
      msg_id: msg.id,
    }).enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) {
      debug('Unauthorized user attempted alac command', {
        user_id: msg.sender.id,
      })
      return
    }

    const commandText = msg.text.trim().split(/\s+/)[0]?.toLowerCase()
    const isRerip = commandText?.includes('rerip')

    const replyMsg = await msg.getReplyTo().catch(() => null)
    const parsed = parseAlacInput(msg.text, replyMsg?.text)
    if (!parsed) {
      debug('Invalid alac input received', { text: msg.text })
      await msg.replyText(
        parseDynamicHtml(
          '<b>Usage:</b><br/>• <code>/alac &lt;apple_music_link | track_id&gt;</code><br/>• Reply to a link with <code>/alac</code><br/>• <code>/alac &lt;link&gt; -f</code> (owner force re-rip)',
        ),
      )
      return
    }

    const isAdmin = auth.isAdmin(msg.sender.id)
    const isForce = parsed.force || isRerip

    if (isForce && !isAdmin) {
      debug('Non-admin requested force re-rip', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml('This command option is restricted to the bot owner.'),
      )
      return
    }

    info('Processing rip request', {
      target_id: parsed.trackId,
      is_album: parsed.isAlbum,
      is_force: isForce,
    })

    // Determine target track IDs (single track vs full album)
    let trackIds: string[] = [parsed.trackId]
    let albumHeader = ''

    if (parsed.isAlbum) {
      try {
        const albumData = await fetchAlbumTracks(parsed.trackId)
        trackIds = albumData.tracks.map((t) => t.id)
        albumHeader = `Album: <b>${albumData.album.title}</b> by <b>${albumData.album.artist}</b> (${trackIds.length} tracks)`
      } catch (err: unknown) {
        const msgText = err instanceof Error ? err.message : String(err)
        error('Failed to resolve album tracks', {
          album_id: parsed.trackId,
          error: msgText,
        })
        await msg.replyText(
          parseDynamicHtml(`Failed to fetch album tracks: ${msgText}`),
        )
        return
      }
    }

    // 1. Batch cache lookup across all target tracks (1 fast DB query)
    const cachedTracksMap = !isForce
      ? await service.findCachedTracks(trackIds)
      : new Map()

    // Process each track in sequence
    for (let index = 0; index < trackIds.length; index++) {
      const trackId = trackIds[index]
      if (!trackId) continue

      using _trackSpan = infoSpan('track_job', {
        track_id: trackId,
        index: index + 1,
        total: trackIds.length,
      }).enter()

      const trackPrefix =
        trackIds.length > 1 ? `[${index + 1}/${trackIds.length}] ` : ''

      const startTime = Date.now()

      // 2. Check Cache (Fast-path)
      if (!isForce) {
        const cached = cachedTracksMap.get(trackId)
        if (cached) {
          try {
            await tg.sendCopy({
              toChatId: msg.chat.id,
              fromChatId: env.DUMP_CHANNEL_ID,
              message: cached.messageId,
              replyTo: msg.id,
            })

            const durationMs = Date.now() - startTime
            info('Cache hit, delivered track from dump channel', {
              track_id: trackId,
              message_id: cached.messageId,
              duration_ms: durationMs,
            })

            await service.logRequest({
              telegramId: msg.sender.id,
              chatId: msg.chat.id,
              appleTrackId: trackId,
              isCacheHit: true,
              durationMs,
              status: 'completed',
            })
            continue
          } catch (err) {
            debug('Cached message delivery failed, falling back to rip', {
              track_id: trackId,
              error: String(err),
            })
            // Remove broken cache entry so future requests re-rip cleanly
            await service.deleteTrack(trackId).catch(() => null)
          }
        }
      }

      // 3. Slow-path: Queue sequential rip job
      debug('Queueing rip job', { track_id: trackId })
      const statusMsg = await msg.replyText(
        parseDynamicHtml(
          `${albumHeader ? `${albumHeader}<br/>` : ''}${trackPrefix}Queued (Position #1)`,
        ),
      )

      let lastUpdate = 0
      let lastText = ''

      const updateStatus = async (text: string, force = false) => {
        const formatted = `${albumHeader ? `${albumHeader}<br/>` : ''}${trackPrefix}${text}`
        const now = Date.now()
        if (formatted === lastText) return
        if (!force && now - lastUpdate < 1200) return
        lastUpdate = now
        lastText = formatted

        await tg
          .editMessage({
            chatId: msg.chat.id,
            message: statusMsg.id,
            text: parseDynamicHtml(formatted),
          })
          .catch(() => null)
      }

      try {
        await queue.enqueue(
          async () => {
            await updateStatus('Connecting to Apple Music server...', true)

            let ripResult: TrackRipResult | null = null

            try {
              ripResult = await ripper.rip(trackId, async (status) => {
                await updateStatus(status)
              })

              await updateStatus('Uploading to Telegram...', true)
              const uploadStart = Date.now()
              info('Uploading lossless track to Telegram dump channel...', {
                track_id: trackId,
                file: ripResult.filePath,
              })

              const dumpMsg = await tg.sendMedia(
                env.DUMP_CHANNEL_ID,
                {
                  type: 'audio',
                  file: Bun.file(ripResult.filePath),
                  title: ripResult.title,
                  performer: ripResult.artist,
                  duration: ripResult.duration,
                },
                {
                  progressCallback: (uploaded, total) => {
                    if (total > 0) {
                      const progressText = formatByteProgress(
                        uploaded,
                        total,
                        10,
                      )
                      updateStatus(
                        `Uploading to Telegram:<br/><code>${progressText}</code>`,
                      )
                    }
                  },
                },
              )

              let fileId = ''
              let fileUniqueId: string | undefined
              if (dumpMsg.media && dumpMsg.media.type === 'audio') {
                fileId = dumpMsg.media.fileId
                fileUniqueId = dumpMsg.media.uniqueFileId
              }

              // Save / update in database
              await service.saveTrack({
                appleTrackId: trackId,
                messageId: dumpMsg.id,
                fileId,
                fileUniqueId,
              })

              // Deliver clean copy to destination chat
              await tg.sendCopy({
                toChatId: msg.chat.id,
                fromChatId: env.DUMP_CHANNEL_ID,
                message: dumpMsg.id,
                replyTo: msg.id,
              })

              // Delete status message
              await tg
                .deleteMessagesById(msg.chat.id, [statusMsg.id])
                .catch(() => null)

              const totalDurationMs = Date.now() - startTime
              info('Track rip and upload complete', {
                track_id: trackId,
                dump_message_id: dumpMsg.id,
                upload_duration_ms: Date.now() - uploadStart,
                total_duration_ms: totalDurationMs,
              })

              // Log success
              await service.logRequest({
                telegramId: msg.sender.id,
                chatId: msg.chat.id,
                appleTrackId: trackId,
                isCacheHit: false,
                durationMs: totalDurationMs,
                status: 'completed',
              })
            } finally {
              // Guaranteed immediate cleanup of local audio file
              if (ripResult?.filePath && existsSync(ripResult.filePath)) {
                try {
                  unlinkSync(ripResult.filePath)
                } catch {}
              }
            }
          },
          {
            onPositionChange: (pos) => {
              debug('Queue position changed', { track_id: trackId, pos })
              updateStatus(`Queued (Position #${pos})`, true)
            },
            onStart: () => {
              debug('Queue job starting execution', { track_id: trackId })
              updateStatus('Connecting to Apple Music server...', true)
            },
          },
        )
      } catch (err: unknown) {
        const errorMsg =
          err instanceof Error ? err.message : 'Unknown error during ripping'
        const totalDurationMs = Date.now() - startTime

        error('Rip job failed', {
          track_id: trackId,
          duration_ms: totalDurationMs,
          error: errorMsg,
          stack: err instanceof Error ? err.stack : undefined,
        })

        await updateStatus(`Failed: ${errorMsg}`, true)

        await service.logRequest({
          telegramId: msg.sender.id,
          chatId: msg.chat.id,
          appleTrackId: trackId,
          isCacheHit: false,
          durationMs: totalDurationMs,
          status: 'failed',
          errorReason: errorMsg,
        })
      }
    }
  })
}
