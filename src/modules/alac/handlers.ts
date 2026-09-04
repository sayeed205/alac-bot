import { existsSync, unlinkSync } from 'node:fs'

import { BotKeyboard, html, type TelegramClient } from '@mtcute/bun'
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
import { formatStatsHtml } from '@/modules/alac/stats.ts'
import { authService, type IAuthService } from '@/modules/auth/service.ts'
import { debug, debugSpan, error, info, infoSpan } from '@/utils/logger.ts'
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
  // Command: /stats
  dp.onNewMessage(filters.command('stats'), async (msg) => {
    using _statsSpan = infoSpan('stats').enter()

    const isAdmin = auth.isAdmin(msg.sender.id)
    if (!isAdmin) {
      debug('Non-admin attempted /stats command', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml('This command is restricted to the bot owner.'),
      )
      return
    }

    info('Stats requested', { user: msg.sender.id })

    const stats = await service.getStats()
    await msg.replyText(parseDynamicHtml(formatStatsHtml(stats)))
  })

  // Command: /search <query>
  dp.onNewMessage(filters.command('search'), async (msg) => {
    using _searchSpan = infoSpan('search').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) {
      debug('Unauthorized user attempted /search', { user_id: msg.sender.id })
      return
    }

    const textParts = msg.text.trim().split(/\s+/)
    const query = textParts.slice(1).join(' ').trim()
    if (!query) {
      await msg.replyText(
        parseDynamicHtml(
          '<b>Usage:</b> <code>/search &lt;track title, artist, or album&gt;</code><br/><br/><i>Searches all lossless tracks already cached in the database for instant download.</i>',
        ),
      )
      return
    }

    const results = await service.searchCachedTracks(query, 8)
    info('Search query', { query, matches: results.length })

    if (results.length === 0) {
      await msg.replyText(
        parseDynamicHtml(
          `No cached tracks found matching "<b>${html.escape(query)}</b>".<br/><br/>💡 Use <code>/alac &lt;apple_music_link&gt;</code> to rip and cache it.`,
        ),
      )
      return
    }

    const formatSecs = (sec: number | null) => {
      if (!sec || sec <= 0) return ''
      const m = Math.floor(sec / 60)
      const s = String(sec % 60).padStart(2, '0')
      return ` • ${m}:${s}`
    }

    const listLines = results.map((t, idx) => {
      const title = html.escape(t.title || `Track ${t.appleTrackId}`)
      const artist = html.escape(t.artist || 'Unknown Artist')
      const quality =
        t.bitDepth && t.sampleRate
          ? ` • ALAC ${t.bitDepth}b/${Math.round(t.sampleRate / 1000)}kHz`
          : ' • ALAC'
      const dur = formatSecs(t.duration)
      return `<b>${idx + 1}. ${title}</b> — ${artist}<br/><i>${quality}${dur}</i>`
    })

    const buttons = results.map((t, idx) => {
      const rawTitle = t.title || `Track ${t.appleTrackId}`
      const shortTitle =
        rawTitle.length > 28 ? `${rawTitle.slice(0, 25)}...` : rawTitle
      return [
        BotKeyboard.callback(
          `🎵 ${idx + 1}. ${shortTitle}`,
          `dl:${t.appleTrackId}`,
        ),
      ]
    })

    buttons.push([BotKeyboard.callback('❌ Close', 'search_close')])

    await msg.replyText(
      parseDynamicHtml(
        `<b>🔍 Found ${results.length} cached track${results.length > 1 ? 's' : ''} for "<i>${html.escape(query)}</i>":</b><br/><br/>` +
          `${listLines.join('<br/><br/>')}<br/><br/>` +
          `<i>Tap a button below for instant delivery:</i>`,
      ),
      { replyMarkup: BotKeyboard.inline(buttons) },
    )
  })

  // Callback query for search download buttons
  dp.onCallbackQuery(
    filters.or(filters.startsWith('dl:'), filters.equals('search_close')),
    async (query) => {
      const data = query.dataStr
      if (!data) return

      if (data === 'search_close') {
        await query.answer()
        await tg
          .deleteMessagesById(query.chat.id, [query.messageId])
          .catch(() => null)
        return
      }

      if (data.startsWith('dl:')) {
        const appleTrackId = data.slice(3)
        using _dlSpan = infoSpan('search_dl').enter()

        const chatId = query.chat.id
        const isAuthed = await auth.isAuthorized(query.user.id, chatId)
        if (!isAuthed) {
          await query.answer({ text: 'Unauthorized', alert: true })
          return
        }

        const cached = await service.findCachedTrack(appleTrackId)
        if (!cached) {
          await query.answer({
            text: 'Track is no longer cached in dump channel.',
            alert: true,
          })
          return
        }

        // Immediately acknowledge button click so Telegram stops spinner
        await query.answer({ text: '⚡ Delivering lossless track from cache!' })

        try {
          await tg.sendCopy({
            toChatId: chatId,
            fromChatId: env.DUMP_CHANNEL_ID,
            message: cached.messageId,
            replyTo: query.messageId,
          })

          info('Delivered cached track', {
            track: cached.title || appleTrackId,
            user: query.user.id,
          })

          await service.logRequest({
            telegramId: query.user.id,
            chatId,
            appleTrackId,
            isCacheHit: true,
            durationMs: 100,
            status: 'completed',
          })
        } catch (err: unknown) {
          error('Failed to deliver cached track via search callback', {
            track_id: appleTrackId,
            error: String(err),
          })
          await query.answer({
            text: 'Failed to retrieve audio from dump channel.',
            alert: true,
          })
        }
      }
    },
  )

  // Command: /alac & /rerip
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

    info('Rip request', {
      target: parsed.trackId,
      type: parsed.isAlbum ? 'album' : 'track',
      force: isForce,
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

      using _trackSpan = debugSpan('track_job', {
        track_id: trackId,
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
              debug('Uploading track to dump channel', {
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

              // Save / update in database with rich audio metadata
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
              info('Track completed', {
                track: `${ripResult.artist} - ${ripResult.title}`,
                time: `${(totalDurationMs / 1000).toFixed(1)}s`,
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
