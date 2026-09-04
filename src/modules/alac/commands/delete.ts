import { html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { info, infoSpan } from '@/utils/logger.ts'

import { parseAlacInput } from '../parser.ts'
import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerDeleteCommand(ctx: CommandContext): void {
  const { dp, tg, service, auth } = ctx

  dp.onNewMessage(filters.command('delete'), async (msg) => {
    using _delSpan = infoSpan('delete').enter()

    if (!auth.isAdmin(msg.sender.id)) {
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.',
        ),
      )
      return
    }

    const replyMsg = await msg.getReplyTo().catch(() => null)
    const parsed = parseAlacInput(msg.text, replyMsg?.text)

    if (!parsed) {
      await msg.replyText(
        parseDynamicHtml(
          '🗑️ <b>Delete Track Usage:</b><br/><br/>' +
            '<blockquote>• <code>/delete &lt;apple_music_link | track_id&gt;</code><br/>' +
            '• Reply to an Apple Music link with <code>/delete</code></blockquote>',
        ),
      )
      return
    }

    const trackId = parsed.trackId
    const cached = await service.findCachedTrack(trackId)

    if (!cached) {
      await msg.replyText(
        parseDynamicHtml(
          `⚠️ <b>Track Not Found:</b> ID <code>${trackId}</code> is not in the database.`,
        ),
      )
      return
    }

    // 1. Delete message from Telegram dump channel
    await tg
      .deleteMessagesById(env.DUMP_CHANNEL_ID, [cached.messageId])
      .catch(() => null)

    // 2. Delete track record from database
    await service.deleteTrack(trackId)

    info('Deleted cached track', {
      track_id: trackId,
      message_id: cached.messageId,
      user: msg.sender.id,
    })

    const title = html.escape(cached.title || `Track ${trackId}`)
    const artist = html.escape(cached.artist || 'Unknown Artist')

    await msg.replyText(
      parseDynamicHtml(
        `🗑️ <b>Track Deleted Successfully</b><br/><br/>` +
          `<blockquote><b>Details:</b><br/>` +
          `• Title: <b>${title}</b> — ${artist}<br/>` +
          `• Apple ID: <code>${trackId}</code><br/>` +
          `• Dump Message: <code>#${cached.messageId}</code><br/>` +
          `• Purged from database & dump channel.</blockquote>`,
      ),
    )
  })
}
