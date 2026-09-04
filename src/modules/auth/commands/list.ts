import { filters } from '@mtcute/dispatcher'

import type { CommandContext } from './types.ts'
import { renderAuthListPage } from './types.ts'

export function registerListCommand(ctx: CommandContext): void {
  const { dp, tg, service } = ctx

  dp.onNewMessage(filters.command('authlist'), async (msg) => {
    if (!service.isAdmin(msg.sender.id)) {
      return
    }

    const textParts = msg.text.trim().split(/\s+/)
    const pageArg = textParts[1] ? Number.parseInt(textParts[1], 10) : 1
    const initialPage = Number.isNaN(pageArg) || pageArg < 1 ? 1 : pageArg

    await renderAuthListPage(
      tg,
      service,
      msg.chat.id,
      undefined,
      initialPage,
      msg.id,
    )
  })

  dp.onCallbackQuery(
    filters.or(filters.startsWith('auth'), filters.equals('noop')),
    async (query) => {
      const data = query.dataStr
      if (!data) return

      if (!service.isAdmin(query.user.id)) {
        await query.answer({ text: 'Unauthorized.', alert: true })
        return
      }

      if (data === 'noop') {
        await query.answer({})
        return
      }

      if (data === 'authclose') {
        await query.answer({})
        await tg
          .deleteMessagesById(query.chat.id, [query.messageId])
          .catch(() => null)
        return
      }

      if (data.startsWith('authpage:')) {
        const page = Number.parseInt(data.split(':')[1] || '1', 10)
        await query.answer({})
        await renderAuthListPage(
          tg,
          service,
          query.chat.id,
          query.messageId,
          page,
        )
      }
    },
  )
}
