import { existsSync, unlinkSync } from 'node:fs'

import { html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { formatDumpCaption } from '@/modules/alac/indexer.ts'
import { fetchAlbumTracks } from '@/modules/alac/itunes.ts'
import { parseAlacInput } from '@/modules/alac/parser.ts'
import type { TrackRipResult } from '@/modules/alac/ripper.ts'
import { debug, debugSpan, error, info, infoSpan } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerRipCommand(ctx: CommandContext): void {
  const { dp, tg, service, ripper, queue, auth } = ctx

  dp.onNewMessage(filters.command(['alac', 'rerip']), async (msg) => {
    using _cmdSpan = infoSpan('alac').enter()

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
          '🎵 <b>Apple Music Lossless Ripper</b><br/><br/>' +
            '<blockquote><b>Usage:</b><br/>' +
            '• <code>/alac &lt;apple_music_link | track_id&gt;</code><br/>' +
            '• Reply to an Apple Music link with <code>/alac</code><br/>' +
            '• <code>/alac &lt;link&gt; -f</code> <i>(force re-rip)</i></blockquote>',
        ),
      )
      return
    }

    const isAdmin = auth.isAdmin(msg.sender.id)
    const isForce = parsed.force || isRerip

    if (isForce && !isAdmin) {
      debug('Non-admin requested force re-rip', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> Force re-rip is restricted to the bot owner.',
        ),
      )
      return
    }

    info('Rip request', {
      target: parsed.trackId,
      type: parsed.isAlbum ? 'album' : 'track',
      force: isForce,
    })

    let trackIds: string[] = [parsed.trackId]
    let albumHeader = ''

    if (parsed.isAlbum) {
      try {
        const albumData = await fetchAlbumTracks(
          parsed.trackId,
          parsed.storefront,
        )
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

    const cachedTracksMap = !isForce
      ? await service.findCachedTracks(trackIds)
      : new Map()

    for (let index = 0; index < trackIds.length; index++) {
      const trackId = trackIds[index]
      if (!trackId) continue

      using _trackSpan = debugSpan('track_job', {
        track_id: trackId,
      }).enter()

      const trackPrefix =
        trackIds.length > 1 ? `[${index + 1}/${trackIds.length}] ` : ''

      const startTime = Date.now()

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
            info('Cache hit: delivered', {
              track_id: trackId,
              time: `${durationMs}ms`,
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
              ripResult = await ripper.rip(
                trackId,
                async (status) => {
                  await updateStatus(status)
                },
                parsed.storefront,
              )

              await updateStatus('📤 <b>Uploading to Telegram...</b>', true)
              debug('Uploading track to dump channel', {
                track_id: trackId,
                file: ripResult.filePath,
              })

              const caption = formatDumpCaption({
                appleTrackId: trackId,
                title: ripResult.title,
                artist: ripResult.artist,
                album: ripResult.album,
                duration: ripResult.duration,
                bitDepth: ripResult.bitDepth,
                sampleRate: ripResult.sampleRate,
                genre: ripResult.genre,
                releaseDate: ripResult.releaseDate,
                trackNumber: ripResult.trackNumber,
                trackCount: ripResult.trackCount,
              })

              const dumpMsg = await tg.sendMedia(
                env.DUMP_CHANNEL_ID,
                {
                  type: 'audio',
                  file: Bun.file(ripResult.filePath),
                  title: ripResult.title,
                  performer: ripResult.artist,
                  duration: ripResult.duration,
                  caption,
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
                        `📤 <b>Uploading to Telegram:</b><br/><code>${progressText}</code>`,
                      )
                    }
                  },
                },
              )

              let fileId = ''
              let fileUniqueId = ''
              if (dumpMsg.media && dumpMsg.media.type === 'audio') {
                fileId = dumpMsg.media.fileId
                fileUniqueId = dumpMsg.media.uniqueFileId
              }

              await service.saveTrack({
                appleTrackId: trackId,
                messageId: dumpMsg.id,
                fileId,
                fileUniqueId,
                title: ripResult.title,
                artist: ripResult.artist,
                album: ripResult.album,
                duration: ripResult.duration,
                bitDepth: ripResult.bitDepth,
                sampleRate: ripResult.sampleRate,
                genre: ripResult.genre,
                releaseDate: ripResult.releaseDate,
                trackNumber: ripResult.trackNumber,
                trackCount: ripResult.trackCount,
              })

              await tg.sendCopy({
                toChatId: msg.chat.id,
                fromChatId: env.DUMP_CHANNEL_ID,
                message: dumpMsg.id,
                replyTo: msg.id,
              })

              await tg
                .deleteMessagesById(msg.chat.id, [statusMsg.id])
                .catch(() => null)

              const totalDurationMs = Date.now() - startTime
              info('Track completed', {
                track: `${ripResult.artist} - ${ripResult.title}`,
                time: `${(totalDurationMs / 1000).toFixed(1)}s`,
              })

              await service.logRequest({
                telegramId: msg.sender.id,
                chatId: msg.chat.id,
                appleTrackId: trackId,
                isCacheHit: false,
                durationMs: totalDurationMs,
                status: 'completed',
              })
            } finally {
              if (ripResult && existsSync(ripResult.filePath)) {
                try {
                  unlinkSync(ripResult.filePath)
                } catch {}
              }
            }
          },
          {
            onPositionChange: (pos) => {
              debug('Queue position changed', { track_id: trackId, pos })
              updateStatus(
                `⏳ <b>In Queue:</b> Position <code>#${pos}</code>`,
                true,
              )
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

        const escapedError = html.escape(errorMsg)
        const failText = `${albumHeader ? `${albumHeader}<br/>` : ''}${trackPrefix}❌ <b>Failed:</b> <code>${escapedError}</code>`

        try {
          await tg.editMessage({
            chatId: msg.chat.id,
            message: statusMsg.id,
            text: parseDynamicHtml(failText),
          })
        } catch {
          await msg.replyText(parseDynamicHtml(failText)).catch(() => null)
        }

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
