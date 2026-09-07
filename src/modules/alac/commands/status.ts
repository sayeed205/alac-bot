import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'
import { renderProgressBar } from '@/utils/progress.ts'
import { editMessageSafe } from '@/utils/telegram.ts'

import { activeJobs } from './rip.ts'
import type { CommandContext, FormattedString } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export const lastStatusMsgByChat = new Map<number, number>()
export const lastRefreshTimeByMsg = new Map<number, number>()

export const STATUS_PAGE_SIZE = 3

export function renderStatusDashboard(page = 1): {
  text: FormattedString
  replyMarkup: ReturnType<typeof BotKeyboard.inline>
} {
  const allJobs = Array.from(activeJobs.values()).filter((j) => !j.completed)

  if (allJobs.length === 0) {
    const text = parseDynamicHtml(
      '🟢 <b>Rip Queue is Idle</b><br/><br/>' +
        '<blockquote>• <b>Status:</b> Idle (No active downloads)<br/>' +
        '• <b>Active Workers:</b> <code>0</code><br/>' +
        '• <b>Queued Jobs:</b> <code>0</code><br/>' +
        '• <b>Ready:</b> Waiting for rip/dump requests.</blockquote>',
    )
    const replyMarkup = BotKeyboard.inline([
      [BotKeyboard.callback('🔄 Refresh', 'status:refresh:1')],
    ])
    return { text, replyMarkup }
  }

  allJobs.sort((a, b) => {
    const aPos = a.queuePosition ?? 0
    const bPos = b.queuePosition ?? 0
    return aPos - bPos
  })

  const totalPages = Math.ceil(allJobs.length / STATUS_PAGE_SIZE) || 1
  const currentPage = Math.max(1, Math.min(page, totalPages))
  const startIndex = (currentPage - 1) * STATUS_PAGE_SIZE
  const pageJobs = allJobs.slice(startIndex, startIndex + STATUS_PAGE_SIZE)

  const activeCount = allJobs.filter(
    (j) => !j.queuePosition || j.queuePosition === 0,
  ).length
  const queuedCount = allJobs.filter(
    (j) => j.queuePosition && j.queuePosition > 0,
  ).length
  const totalRemainingTracks = allJobs.reduce((sum, j) => {
    const done = j.cachedCount + j.rippedCount + j.failedCount
    return sum + Math.max(0, j.totalTracks - done)
  }, 0)

  let htmlContent = '📊 <b>Rip Queue & System Status</b><br/><br/>'
  htmlContent += `<blockquote>• <b>Active Tasks:</b> <code>${activeCount}</code> | <b>Queued:</b> <code>${queuedCount}</code><br/>`
  htmlContent += `• <b>Remaining Tracks:</b> <code>${totalRemainingTracks}</code></blockquote><br/>`

  for (let i = 0; i < pageJobs.length; i++) {
    const job = pageJobs[i]
    const globalIdx = startIndex + i + 1
    const isQueued = job.queuePosition && job.queuePosition > 0
    const completed = job.cachedCount + job.rippedCount + job.failedCount
    const progressBar = renderProgressBar(completed, job.totalTracks, 10)
    const elapsedSec = Math.max(
      0,
      Math.floor((Date.now() - (job.startTime || Date.now())) / 1000),
    )
    const elapsedStr =
      elapsedSec >= 60
        ? `${Math.floor(elapsedSec / 60)}m ${elapsedSec % 60}s`
        : `${elapsedSec}s`

    htmlContent += `<b>[#${globalIdx}] ${job.jobHeader || 'Lossless Rip Job'}</b><br/>`
    htmlContent += '<blockquote>'
    if (isQueued) {
      htmlContent += `• <b>State:</b> ⏳ In Queue (Position #<code>${job.queuePosition}</code>)<br/>`
      htmlContent += `• <b>Total Tracks:</b> <code>${job.totalTracks}</code><br/>`
    } else {
      const action = job.activeActionText || '⚡ Processing...'
      htmlContent += `• <b>Action:</b> ${action}<br/>`
      htmlContent += `• <b>Progress:</b> <code>${progressBar}</code> (<code>${completed}/${job.totalTracks}</code>)<br/>`
      htmlContent += `• <b>Delivered:</b> ⚡ <code>${job.cachedCount}</code> cached • 🎵 <code>${job.rippedCount}</code> ripped`
      if (job.failedCount > 0) {
        htmlContent += ` • ❌ <code>${job.failedCount}</code> failed`
      }
      htmlContent += '<br/>'
      htmlContent += `• <b>Elapsed:</b> <code>${elapsedStr}</code><br/>`
    }
    htmlContent += `• <b>Requester:</b> ${html.escape(job.userName || `User ${job.userId}`)}`
    htmlContent += '</blockquote><br/>'
  }

  const cancelButtons: ReturnType<typeof BotKeyboard.callback>[] = []
  for (let i = 0; i < pageJobs.length; i++) {
    const job = pageJobs[i]
    const globalIdx = startIndex + i + 1
    cancelButtons.push(
      BotKeyboard.callback(
        `❌ Stop #${globalIdx}`,
        `status:cancel:${job.id}:${currentPage}`,
      ),
    )
  }

  const navRow: ReturnType<typeof BotKeyboard.callback>[] = []
  if (currentPage > 1) {
    navRow.push(
      BotKeyboard.callback('◀️ Prev', `status:page:${currentPage - 1}`),
    )
  }
  navRow.push(
    BotKeyboard.callback(
      `🔄 Refresh (${currentPage}/${totalPages})`,
      `status:refresh:${currentPage}`,
    ),
  )
  if (currentPage < totalPages) {
    navRow.push(
      BotKeyboard.callback('Next ▶️', `status:page:${currentPage + 1}`),
    )
  }

  const rows: ReturnType<typeof BotKeyboard.callback>[][] = []
  for (let r = 0; r < cancelButtons.length; r += 2) {
    rows.push(cancelButtons.slice(r, r + 2))
  }
  rows.push(navRow)

  return {
    text: parseDynamicHtml(htmlContent),
    replyMarkup: BotKeyboard.inline(rows),
  }
}

export function registerStatusCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx

  dp.onNewMessage(filters.command('status'), async (msg) => {
    using _statusSpan = infoSpan('status').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const prevMsgId = lastStatusMsgByChat.get(msg.chat.id)
    if (prevMsgId) {
      try {
        await tg.deleteMessagesById(msg.chat.id, [prevMsgId])
      } catch {}
    }

    const { text, replyMarkup } = renderStatusDashboard(1)
    const sent = await msg.replyText(text, { replyMarkup })
    lastStatusMsgByChat.set(msg.chat.id, sent.id)
  })

  dp.onCallbackQuery(
    filters.regex(/^status:(refresh|page|cancel)(?::(.*))?$/),
    async (cq) => {
      const match = cq.match
      const action = match[1]
      const param = match[2]

      const chatId = cq.chat.id
      const messageId = cq.messageId

      const isAuthed = await auth.isAuthorized(cq.user.id, chatId)
      if (!isAuthed) {
        await cq.answer({ text: '⛔ Unauthorized', alert: true })
        return
      }

      if (!messageId) {
        await cq.answer({})
        return
      }

      if (action === 'refresh') {
        const page = Number.parseInt(param || '1', 10) || 1
        const now = Date.now()
        const lastRefresh = lastRefreshTimeByMsg.get(messageId) ?? 0

        if (now - lastRefresh < 2500) {
          await cq.answer({ text: '⏳ Status is already up to date!' })
          return
        }
        lastRefreshTimeByMsg.set(messageId, now)

        const { text, replyMarkup } = renderStatusDashboard(page)
        await editMessageSafe(tg, {
          chatId,
          message: messageId,
          text,
          replyMarkup,
          block: false,
        })
        await cq.answer({ text: '✅ Refreshed!' })
        return
      }

      if (action === 'page') {
        const page = Number.parseInt(param || '1', 10) || 1
        const { text, replyMarkup } = renderStatusDashboard(page)
        await editMessageSafe(tg, {
          chatId,
          message: messageId,
          text,
          replyMarkup,
          block: false,
        })
        await cq.answer({})
        return
      }

      if (action === 'cancel') {
        const [jobId, pageStr] = (param || '').split(':')
        const page = Number.parseInt(pageStr || '1', 10) || 1
        const job = activeJobs.get(jobId)

        if (!job || job.completed) {
          await cq.answer({
            text: 'ℹ️ Job has already completed or does not exist.',
          })
          const { text, replyMarkup } = renderStatusDashboard(page)
          await editMessageSafe(tg, {
            chatId,
            message: messageId,
            text,
            replyMarkup,
            block: false,
          })
          return
        }

        const isAdmin = auth.isAdmin(cq.user.id)
        const isRequester = cq.user.id === job.userId

        if (!isAdmin && !isRequester) {
          await cq.answer({
            text: '⛔ You cannot cancel this download.',
            alert: true,
          })
          return
        }

        job.isCancelled = true
        job.cancelledBy =
          cq.user.displayName ||
          (cq.user.username ? `@${cq.user.username}` : `User ${cq.user.id}`)
        job.controller.abort()

        await cq.answer({ text: '🛑 Download cancelled.' })

        const { text, replyMarkup } = renderStatusDashboard(page)
        await editMessageSafe(tg, {
          chatId,
          message: messageId,
          text,
          replyMarkup,
          block: false,
        })
      }
    },
  )
}
