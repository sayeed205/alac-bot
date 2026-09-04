import { filters } from '@mtcute/dispatcher'

import { formatStatsHtml } from '@/modules/alac/stats.ts'
import { debug, info, infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerStatsCommand(ctx: CommandContext): void {
  const { dp, service, auth } = ctx

  dp.onNewMessage(filters.command('stats'), async (msg) => {
    using _statsSpan = infoSpan('stats').enter()

    const isAdmin = auth.isAdmin(msg.sender.id)
    if (!isAdmin) {
      debug('Non-admin attempted /stats command', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.',
        ),
      )
      return
    }

    info('Stats requested', { user: msg.sender.id })

    const stats = await service.getStats()
    await msg.replyText(parseDynamicHtml(formatStatsHtml(stats)))
  })
}
