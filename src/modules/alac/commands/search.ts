import { existsSync, unlinkSync } from 'node:fs'

import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { formatDumpCaption } from '@/modules/alac/indexer.ts'
import { searchItunesCatalog } from '@/modules/alac/itunes.ts'
import type { TrackRipResult } from '@/modules/alac/ripper.ts'
import { settingsService as defaultSettingsService } from '@/modules/settings/service.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerSearchCommand(ctx: CommandContext): void {
  const { dp, tg, service, ripper, queue, auth } = ctx
  const settings = ctx.settings ?? defaultSettingsService

  dp.onNewMessage(filters.command('search'), async (msg) => {
    using _searchSpan = infoSpan('search').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) {
      debug('Unauthorized user attempted /search', { user_id: msg.sender.id })
      return
    }

    const isAdmin = auth.isAdmin(msg.sender.id)
    if (!settings.canServeCache(isAdmin)) {
      await msg.replyText(
        parseDynamicHtml(
          '⚠️ <b>Service is temporarily paused for maintenance.</b><br/>' +
            'Please check back later.',
        ),
      )
      return
    }

    const textParts = msg.text.trim().split(/\s+/)
    const query = textParts.slice(1).join(' ').trim()
    if (!query) {
      await msg.replyText(
        parseDynamicHtml(
          '🔍 <b>Search Music:</b><br/><br/>' +
            '<blockquote><b>Usage:</b> <code>/search &lt;track title or artist&gt;</code><br/>' +
            '<i>Searches both local lossless cache and the live Apple Music catalog.</i></blockquote>',
        ),
      )
      return
    }

    const [cachedResults, liveResults] = await Promise.all([
      service.searchCachedTracks(query, 5),
      searchItunesCatalog(query, 10).catch((err: unknown) => {
        debug('Live Apple Music catalog search failed', {
          query,
          error: String(err),
        })
        return []
      }),
    ])

    if (cachedResults.length === 0 && liveResults.length === 0) {
      await msg.replyText(
        parseDynamicHtml(
          `🔍 No tracks found for "<b>${html.escape(query)}</b>". Try refining your search query!`,
        ),
      )
      return
    }

    const sections: string[] = []
    const keyboardButtons: Parameters<typeof BotKeyboard.inline>[0] = []

    if (cachedResults.length > 0) {
      const cachedLines = cachedResults.map((t, i) => {
        const quality =
          t.bitDepth && t.sampleRate
            ? ` [${t.bitDepth}-bit/${(t.sampleRate / 1000).toFixed(1)}kHz]`
            : ''
        return `${i + 1}. <b>${html.escape(t.title)}</b> — <i>${html.escape(t.artist)}</i><code>${quality}</code>`
      })
      sections.push(
        `⚡ <b>Instant Lossless Cache:</b><br/><blockquote>${cachedLines.join('<br/>')}</blockquote>`,
      )

      for (let i = 0; i < cachedResults.length; i++) {
        const t = cachedResults[i]
        if (!t) continue
        const rawTitle = `${t.artist} - ${t.title}`
        const shortTitle =
          rawTitle.length > 28 ? `${rawTitle.slice(0, 25)}...` : rawTitle
        keyboardButtons.push([
          BotKeyboard.callback(
            `⚡ ${i + 1}. ${shortTitle}`,
            `dl:${t.appleTrackId}`,
          ),
        ])
      }
    }

    const cachedIds = new Set(cachedResults.map((t) => t.appleTrackId))
    const uncachedLiveResults = liveResults.filter((t) => !cachedIds.has(t.id))

    if (uncachedLiveResults.length > 0) {
      const catalogStartIndex = cachedResults.length + 1
      const liveLines = uncachedLiveResults.map((t, i) => {
        return `${catalogStartIndex + i}. <b>${html.escape(t.title)}</b> — <i>${html.escape(t.artist)}</i>`
      })
      sections.push(
        `🎵 <b>Apple Music Catalog:</b><br/><blockquote>${liveLines.join('<br/>')}</blockquote>`,
      )

      for (let i = 0; i < uncachedLiveResults.length; i++) {
        const t = uncachedLiveResults[i]
        if (!t) continue
        const rawTitle = `${t.artist} - ${t.title}`
        const shortTitle =
          rawTitle.length > 28 ? `${rawTitle.slice(0, 25)}...` : rawTitle
        keyboardButtons.push([
          BotKeyboard.callback(
            `🎵 ${catalogStartIndex + i}. ${shortTitle}`,
            `rip:${t.id}`,
          ),
        ])
      }
    }

    keyboardButtons.push([BotKeyboard.callback('❌ Close', 'search_close')])

    await msg.replyText(
      parseDynamicHtml(
        `🔍 <b>Search results for "<i>${html.escape(query)}</i>":</b><br/><br/>` +
          sections.join('<br/><br/>') +
          '<br/><br/><i>Tap ⚡ for instant cache delivery or 🎵 to rip ALAC lossless:</i>',
      ),
      { replyMarkup: BotKeyboard.inline(keyboardButtons) },
    )
  })

  dp.onCallbackQuery(
    filters.or(
      filters.startsWith('dl:'),
      filters.startsWith('rip:'),
      filters.equals('search_close'),
    ),
    async (query) => {
      const data = query.dataStr
      if (!data) return

      if (data === 'search_close') {
        await query.answer({})
        await tg
          .deleteMessagesById(query.chat.id, [query.messageId])
          .catch(() => null)
        return
      }

      if (data.startsWith('dl:')) {
        const appleTrackId = data.slice(3)
        const chatId = query.chat.id
        const isAuthed = await auth.isAuthorized(query.user.id, chatId)
        if (!isAuthed) {
          await query.answer({ text: 'Unauthorized', alert: true })
          return
        }

        const isAdmin = auth.isAdmin(query.user.id)
        if (!settings.canServeCache(isAdmin)) {
          await query.answer({
            text: '⚠️ Service is temporarily paused for maintenance.',
            alert: true,
          })
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

        await query.answer({ text: '⚡ Delivering lossless track from cache!' })

        try {
          await tg.sendCopy({
            toChatId: chatId,
            fromChatId: env.DUMP_CHANNEL_ID,
            message: cached.messageId,
          })

          await tg
            .deleteMessagesById(chatId, [query.messageId])
            .catch(() => null)

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
        return
      }

      if (data.startsWith('rip:')) {
        const appleTrackId = data.slice(4)
        using _ripSpan = infoSpan('search_rip').enter()

        const chatId = query.chat.id
        const isAuthed = await auth.isAuthorized(query.user.id, chatId)
        if (!isAuthed) {
          await query.answer({ text: 'Unauthorized', alert: true })
          return
        }

        const isAdmin = auth.isAdmin(query.user.id)

        // Check if already in cache
        const cached = await service.findCachedTrack(appleTrackId)
        if (cached) {
          if (!settings.canServeCache(isAdmin)) {
            await query.answer({
              text: '⚠️ Service is temporarily paused for maintenance.',
              alert: true,
            })
            return
          }

          await query.answer({
            text: '⚡ Already cached! Delivering track...',
          })
          await tg.sendCopy({
            toChatId: chatId,
            fromChatId: env.DUMP_CHANNEL_ID,
            message: cached.messageId,
          })
          await tg
            .deleteMessagesById(chatId, [query.messageId])
            .catch(() => null)
          return
        }

        // Live ripping check
        if (!settings.canRipLive(isAdmin)) {
          await query.answer({
            text: '⚠️ Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now.',
            alert: true,
          })
          return
        }

        await query.answer({ text: '⏳ Queuing lossless rip...' })

        const statusMsg = await tg.sendText(
          chatId,
          parseDynamicHtml(
            `⏳ <b>Queuing track ${appleTrackId} for ripping...</b>`,
          ),
          { replyTo: query.messageId },
        )

        let lastUpdate = 0
        const updateStatus = async (status: string, force = false) => {
          const now = Date.now()
          if (!force && now - lastUpdate < 1500) return
          lastUpdate = now
          await tg
            .editMessage({
              chatId,
              message: statusMsg.id,
              text: parseDynamicHtml(
                `🎵 <b>Ripping track:</b> <code>${appleTrackId}</code><br/>Status: ${status}`,
              ),
            })
            .catch(() => null)
        }

        const startTime = Date.now()

        try {
          await queue.enqueue(
            async () => {
              await updateStatus('Connecting to Apple Music server...', true)

              let ripResult: TrackRipResult | null = null

              try {
                ripResult = await ripper.rip(appleTrackId, async (status) => {
                  await updateStatus(status)
                })

                await updateStatus('📤 <b>Uploading to Telegram...</b>', true)

                const caption = formatDumpCaption({
                  appleTrackId,
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
                    silent: true,
                    progressCallback: (uploaded, total) => {
                      if (total > 0) {
                        const progressText = formatByteProgress(
                          uploaded,
                          total,
                          10,
                        )
                        updateStatus(
                          `📤 <b>Uploading:</b> <code>${progressText}</code>`,
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
                  appleTrackId,
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
                  toChatId: chatId,
                  fromChatId: env.DUMP_CHANNEL_ID,
                  message: dumpMsg.id,
                })

                await tg
                  .deleteMessagesById(chatId, [statusMsg.id, query.messageId])
                  .catch(() => null)

                const totalDurationMs = Date.now() - startTime
                info('Search track ripped and delivered', {
                  track: `${ripResult.artist} - ${ripResult.title}`,
                  time: `${(totalDurationMs / 1000).toFixed(1)}s`,
                })

                await service.logRequest({
                  telegramId: query.user.id,
                  chatId,
                  appleTrackId,
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
              onPositionChange: (pos: number) => {
                updateStatus(
                  `⏳ <b>In Queue:</b> Position <code>#${pos}</code>`,
                  true,
                )
              },
              onStart: () => {
                debug('Search track rip job started', {
                  track_id: appleTrackId,
                })
              },
            },
          )
        } catch (err: unknown) {
          const errorMsg =
            err instanceof Error ? err.message : 'Unknown error during ripping'
          error('Search rip job failed', {
            track_id: appleTrackId,
            error: errorMsg,
          })

          const escapedError = html.escape(errorMsg)
          await tg
            .editMessage({
              chatId,
              message: statusMsg.id,
              text: parseDynamicHtml(
                `⚠️ <b>Rip failed for track ${appleTrackId}:</b><br/><code>${escapedError}</code>`,
              ),
            })
            .catch(() => null)

          await service.logRequest({
            telegramId: query.user.id,
            chatId,
            appleTrackId,
            isCacheHit: false,
            durationMs: Date.now() - startTime,
            status: 'failed',
            errorReason: errorMsg,
          })
        }
      }
    },
  )
}
