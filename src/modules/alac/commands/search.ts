import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerSearchCommand(ctx: CommandContext): void {
  const { dp, tg, service, auth } = ctx

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
          '🔍 <b>Search Cached Tracks:</b><br/><br/>' +
            '<blockquote><b>Usage:</b> <code>/search &lt;track title, artist, or album&gt;</code><br/>' +
            '<i>Searches lossless tracks already cached in the database for instant download.</i></blockquote>',
        ),
      )
      return
    }

    const results = await service.searchCachedTracks(query, 8)
    info('Search query', { query, matches: results.length })

    if (results.length === 0) {
      await msg.replyText(
        parseDynamicHtml(
          `🔍 <b>No cached tracks found</b> matching "<i>${html.escape(query)}</i>".<br/><br/>` +
            `💡 Use <code>/alac &lt;apple_music_link&gt;</code> to rip and cache it.`,
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
        `🔍 <b>Found ${results.length} cached track${results.length > 1 ? 's' : ''} for "<i>${html.escape(query)}</i>":</b><br/><br/>` +
          `<blockquote>${listLines.join('<br/><br/>')}</blockquote><br/>` +
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
}
