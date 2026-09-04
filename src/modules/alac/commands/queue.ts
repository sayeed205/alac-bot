import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerQueueCommand(ctx: CommandContext): void {
  const { dp, queue, auth } = ctx

  dp.onNewMessage(filters.command('queue'), async (msg) => {
    using _queueSpan = infoSpan('queue').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const pending = queue.getPendingCount()
    const isBusy = queue.isProcessing()

    if (!isBusy && pending === 0) {
      await msg.replyText(
        parseDynamicHtml(
          '🟢 <b>Rip Queue is Idle</b><br/><br/>' +
            '<blockquote>• Active Workers: <code>0</code><br/>' +
            '• Pending Jobs: <code>0</code><br/>' +
            '• Ready to process new rip requests.</blockquote>',
        ),
      )
      return
    }

    await msg.replyText(
      parseDynamicHtml(
        '🔄 <b>Rip Queue Status</b><br/><br/>' +
          `<blockquote>• Worker Status: <b>${isBusy ? 'Active' : 'Idle'}</b><br/>` +
          `• Pending in Queue: <code>${pending}</code> task${pending === 1 ? '' : 's'}<br/>` +
          '• Tasks are processed sequentially.</blockquote>',
      ),
    )
  })
}
