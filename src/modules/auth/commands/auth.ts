import { html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import type { CommandContext } from './types.ts'
import { getPeerLink, parseDynamicHtml, resolveTarget } from './types.ts'

export function registerAuthCommand(ctx: CommandContext): void {
  const { dp, tg, service } = ctx

  dp.onNewMessage(filters.command('auth'), async (msg) => {
    if (!service.isAdmin(msg.sender.id)) {
      return
    }

    const target = await resolveTarget(msg, tg)
    if (!target) {
      await msg.answerText(
        parseDynamicHtml(
          '🔑 <b>Authorization Usage:</b><br/><br/>' +
            '<blockquote>• Reply to a message with <code>/auth</code><br/>' +
            '• <code>/auth &lt;id | @username&gt;</code><br/>' +
            '• Send <code>/auth</code> inside a group to authorize the whole group</blockquote>',
        ),
      )
      return
    }

    const { link, name } = await getPeerLink(tg, target.id, target.name)
    const { newlyAdded } = await service.authorize(target.id, name)
    const escapedName = html.escape(name)
    const icon = target.isUser ? '👤' : '👥'
    const typeLabel = target.isUser ? 'User' : 'Group'

    await msg.answerText(
      parseDynamicHtml(
        `✅ <b>${newlyAdded ? 'Authorized' : 'Updated'} ${typeLabel}:</b> ${icon} <a href="${link}">${escapedName}</a> (<code>${target.id}</code>)`,
      ),
    )
  })
}
