import { html, type Message } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import {
  formatIndexSummaryHtml,
  indexDumpChannel,
} from '@/modules/alac/indexer.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerIndexCommand(ctx: CommandContext): void {
  const { dp, tg, service, auth } = ctx
  let isIndexing = false

  dp.onNewMessage(filters.command('index'), async (msg) => {
    using _indexSpan = infoSpan('index').enter()

    if (!auth.isAdmin(msg.sender.id)) {
      debug('Non-admin attempted /index command', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.',
        ),
      )
      return
    }

    if (isIndexing) {
      await msg.replyText(
        parseDynamicHtml('⚠️ <b>Dump channel sync is already in progress.</b>'),
      )
      return
    }

    isIndexing = true
    let statusMsg: Message | null = null

    try {
      info('Starting dump channel index', { user: msg.sender.id })
      statusMsg = await msg.replyText(
        parseDynamicHtml('🔄 <b>Initializing Dump Channel Sync...</b>'),
      )

      let lastUpdate = Date.now()
      const summary = await indexDumpChannel(
        tg,
        service,
        env.DUMP_CHANNEL_ID,
        async (scanned, synced) => {
          const now = Date.now()
          if (now - lastUpdate >= 2000 && statusMsg) {
            lastUpdate = now
            await tg
              .editMessage({
                chatId: statusMsg.chat.id,
                message: statusMsg.id,
                text: parseDynamicHtml(
                  `🔄 <b>Syncing with Dump Channel...</b><br/><br/>` +
                    `• Scanned: <code>${scanned}</code> messages<br/>` +
                    `• Synced: <code>${synced}</code> tracks`,
                ),
              })
              .catch(() => null)
          }
        },
      )

      info('Dump channel index completed', {
        scanned: summary.scanned,
        synced: summary.synced,
        pruned: summary.pruned,
        duration_ms: summary.durationMs,
      })

      const finalHtml = formatIndexSummaryHtml(summary)
      if (statusMsg) {
        await tg
          .editMessage({
            chatId: statusMsg.chat.id,
            message: statusMsg.id,
            text: finalHtml,
          })
          .catch(() => msg.replyText(finalHtml))
      } else {
        await msg.replyText(finalHtml)
      }
    } catch (err) {
      error('Dump channel index failed', { error: String(err) })
      const errText = `❌ <b>Indexing failed:</b> <code>${html.escape(String(err))}</code>`
      if (statusMsg) {
        await tg
          .editMessage({
            chatId: statusMsg.chat.id,
            message: statusMsg.id,
            text: parseDynamicHtml(errText),
          })
          .catch(() => msg.replyText(parseDynamicHtml(errText)))
      } else {
        await msg.replyText(parseDynamicHtml(errText))
      }
    } finally {
      isIndexing = false
    }
  })
}
