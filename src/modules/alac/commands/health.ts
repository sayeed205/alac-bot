import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'

import { getMirrorEndpoint } from '../manifest.ts'
import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)

  const parts: string[] = []
  if (days > 0) parts.push(`${days}d`)
  if (hours > 0) parts.push(`${hours}h`)
  if (minutes > 0) parts.push(`${minutes}m`)
  parts.push(`${secs}s`)
  return parts.join(' ')
}

export function registerHealthCommand(ctx: CommandContext): void {
  const { dp, tg, service, queue, auth } = ctx

  dp.onNewMessage(filters.command(['ping', 'health']), async (msg) => {
    using _healthSpan = infoSpan('health').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const startPing = Date.now()
    const statusMsg = await msg.replyText(
      parseDynamicHtml('🏓 <b>Testing system health...</b>'),
    )
    const tgLatency = Date.now() - startPing

    // Measure DB latency
    const dbStart = Date.now()
    let dbStatus = 'Operational'
    try {
      await service.getStats()
    } catch {
      dbStatus = 'Degraded'
    }
    const dbLatency = Date.now() - dbStart

    // Check ALAC mirror availability
    let mirrorStatus = 'Online'
    const mirrorStart = Date.now()
    try {
      const { mirrorUrl } = await getMirrorEndpoint()
      const resp = await fetch(mirrorUrl, {
        method: 'HEAD',
        signal: AbortSignal.timeout(4000),
      }).catch(() => null)
      if (!resp) {
        mirrorStatus = 'Unreachable'
      }
    } catch {
      mirrorStatus = 'Unavailable'
    }
    const mirrorLatency = Date.now() - mirrorStart

    // Memory & Uptime
    const memMb = (process.memoryUsage().rss / (1024 * 1024)).toFixed(1)
    const uptimeStr = formatUptime(process.uptime())

    const queueState = queue.isProcessing()
      ? `Processing (${queue.getPendingCount()} queued)`
      : 'Idle'

    const healthCard =
      '🏓 <b>Pong! System Health</b><br/><br/>' +
      `<blockquote><b>⚡ Latencies & Services:</b><br/>` +
      `• Telegram API: <code>${tgLatency}ms</code><br/>` +
      `• Database: <b>${dbStatus}</b> (<code>${dbLatency}ms</code>)<br/>` +
      `• ALAC Mirror: <b>${mirrorStatus}</b> (<code>${mirrorLatency}ms</code>)</blockquote><br/>` +
      `<blockquote><b>🖥️ System Metrics:</b><br/>` +
      `• Uptime: <code>${uptimeStr}</code><br/>` +
      `• RAM (RSS): <code>${memMb} MB</code><br/>` +
      `• Rip Worker: <code>${queueState}</code></blockquote>`

    await tg
      .editMessage({
        chatId: statusMsg.chat.id,
        message: statusMsg.id,
        text: parseDynamicHtml(healthCard),
      })
      .catch(() => null)
  })
}
