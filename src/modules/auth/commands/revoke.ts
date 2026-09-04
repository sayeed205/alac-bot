import { html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import type { CommandContext } from './types.ts'
import { getPeerLink, parseDynamicHtml, resolveTarget } from './types.ts'

export function registerRevokeCommand(ctx: CommandContext): void {
  const { dp, tg, service } = ctx

  dp.onNewMessage(filters.command(['revoke', 'unauth']), async (msg) => {
    if (!service.isAdmin(msg.sender.id)) {
      return
    }

    const target = await resolveTarget(msg, tg)
    if (!target) {
      await msg.answerText(
        parseDynamicHtml(
          '🔒 <b>Revocation Usage:</b><br/><br/>' +
            '<blockquote>• Reply to a message with <code>/revoke</code><br/>' +
            '• <code>/revoke &lt;id | @username&gt;</code><br/>' +
            '• Send <code>/revoke</code> inside a group to revoke the group</blockquote>',
        ),
      )
      return
    }

    const { link, name } = await getPeerLink(tg, target.id, target.name)
    const { revoked } = await service.revoke(target.id)
    const escapedName = html.escape(name)

    if (revoked) {
      await msg.answerText(
        parseDynamicHtml(
          `🗑️ <b>Revoked access for:</b> <a href="${link}">${escapedName}</a> (<code>${target.id}</code>)`,
        ),
      )
    } else {
      await msg.answerText(
        parseDynamicHtml(
          `⚠️ <b>Not Found:</b> ID <code>${target.id}</code> was not in the authorized list.`,
        ),
      )
    }
  })
}
