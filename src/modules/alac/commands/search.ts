import { existsSync, unlinkSync } from 'node:fs'

import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { formatDumpCaption } from '@/modules/alac/indexer.ts'
import { searchItunesCatalog } from '@/modules/alac/itunes.ts'
import type { TrackRipResult } from '@/modules/alac/ripper.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerSearchCommand(ctx: CommandContext): void {
  const { dp, tg, service, ripper, queue, auth } = ctx

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
          '🔍 <b>Search Music:</b><br/><br/>' +
            '<blockquote><b>Usage:</b> <code>/search &lt;track title or artist&gt;</code><br/>' +
            '<i>Searches both local lossless cache and the live Apple Music catalog.</i></blockquote>',
        ),
      )
      return
    }

    const [cachedResults, liveResults] = await Promise.all([
      service.searchCachedTracks(query, 5),
      searchItunesCatalog(query, 5).catch((err: unknown) => {
        debug('Live Apple Music catalog search failed', {
          query,
          error: String(err),
        })
        return []
      }),
    ])

    // Deduplicate: exclude catalog items that already exist in local cached results
    const cachedIds = new Set(cachedResults.map((c) => c.appleTrackId))
    const catalogResults = liveResults.filter((item) => !cachedIds.has(item.id))

    info('Search query results', {
      query,
      cachedMatches: cachedResults.length,
      catalogMatches: catalogResults.length,
    })

    if (cachedResults.length === 0 && catalogResults.length === 0) {
      await msg.replyText(
        parseDynamicHtml(
          `🔍 <b>No tracks found</b> matching "<i>${html.escape(query)}</i>" in local cache or Apple Music catalog.<br/><br/>` +
            '💡 Try searching with different keywords or paste a direct Apple Music link with <code>/alac</code>.',
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

    const sections: string[] = []
    const keyboardButtons: ReturnType<typeof BotKeyboard.callback>[][] = []

    if (cachedResults.length > 0) {
      const cachedLines = cachedResults.map((t, idx) => {
        const title = html.escape(t.title || `Track ${t.appleTrackId}`)
        const artist = html.escape(t.artist || 'Unknown Artist')
        const quality =
          t.bitDepth && t.sampleRate
            ? ` • ALAC ${t.bitDepth}b/${Math.round(t.sampleRate / 1000)}kHz`
            : ' • ALAC'
        const dur = formatSecs(t.duration)
        return `<b>${idx + 1}. ${title}</b> — ${artist}<br/><i>${quality}${dur}</i>`
      })

      sections.push(
        `⚡ <b>Cached in Database:</b><br/><blockquote>${cachedLines.join('<br/><br/>')}</blockquote>`,
      )

      for (let i = 0; i < cachedResults.length; i++) {
        const t = cachedResults[i]
        if (!t) continue
        const rawTitle = t.title || `Track ${t.appleTrackId}`
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

    if (catalogResults.length > 0) {
      const catalogLines = catalogResults.map((t, idx) => {
        const title = html.escape(t.title || `Track ${t.id}`)
        const artist = html.escape(t.artist || 'Unknown Artist')
        const dur = formatSecs(t.duration)
        return `<b>${idx + 1}. ${title}</b> — ${artist}<i>${dur}</i>`
      })

      sections.push(
        `🎵 <b>Apple Music Catalog:</b><br/><blockquote>${catalogLines.join('<br/><br/>')}</blockquote>`,
      )

      for (let i = 0; i < catalogResults.length; i++) {
        const t = catalogResults[i]
        if (!t) continue
        const rawTitle = t.title || `Track ${t.id}`
        const shortTitle =
          rawTitle.length > 28 ? `${rawTitle.slice(0, 25)}...` : rawTitle
        keyboardButtons.push([
          BotKeyboard.callback(`🎵 ${i + 1}. ${shortTitle}`, `rip:${t.id}`),
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

        // Check if already in cache
        const cached = await service.findCachedTrack(appleTrackId)
        if (cached) {
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

        await query.answer({ text: '⏳ Queuing lossless rip...' })

        const statusMsg = await tg.sendText(
          chatId,
          parseDynamicHtml(
            `⏳ <b>Queuing track ${appleTrackId} for ripping...</b>`,
          ),
          { replyTo: query.messageId },
        )

        let lastUpdate = 0
        let lastText = ''

        const updateStatus = async (text: string, force = false) => {
          const now = Date.now()
          if (text === lastText) return
          if (!force && now - lastUpdate < 1200) return
          lastUpdate = now
          lastText = text

          await tg
            .editMessage({
              chatId,
              message: statusMsg.id,
              text: parseDynamicHtml(text),
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
              onPositionChange: (pos) => {
                updateStatus(
                  `⏳ <b>In Queue:</b> Position <code>#${pos}</code>`,
                  true,
                )
              },
              onStart: () => {
                updateStatus('Connecting to Apple Music server...', true)
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
                `❌ <b>Failed:</b> <code>${escapedError}</code>`,
              ),
            })
            .catch(() => null)
        }
      }
    },
  )
}
