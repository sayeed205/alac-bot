import { existsSync, unlinkSync } from 'node:fs'

import { html, type TelegramClient } from '@mtcute/bun'
import { type Dispatcher, filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { parseAlacInput } from '@/modules/alac/parser.ts'
import {
  ripQueue as defaultQueue,
  type IRipQueue,
} from '@/modules/alac/queue.ts'
import { defaultRipper, type ITrackRipper } from '@/modules/alac/ripper.ts'
import {
  alacService as defaultService,
  type IAlacService,
} from '@/modules/alac/service.ts'
import { authService, type IAuthService } from '@/modules/auth/service.ts'

function parseDynamicHtml(content: string) {
  return html([content] as unknown as TemplateStringsArray)
}

export function registerAlacHandlers(
  dp: Dispatcher<TelegramClient>,
  tg: TelegramClient,
  service: IAlacService = defaultService,
  ripper: ITrackRipper = defaultRipper,
  queue: IRipQueue = defaultQueue,
  auth: IAuthService = authService,
) {
  dp.onNewMessage(filters.command(['alac', 'rerip']), async (msg) => {
    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) {
      return
    }

    const commandText = msg.text.trim().split(/\s+/)[0]?.toLowerCase()
    const isRerip = commandText?.includes('rerip')

    const replyMsg = await msg.getReplyTo().catch(() => null)
    const parsed = parseAlacInput(msg.text, replyMsg?.text)
    if (!parsed) {
      await msg.replyText(
        parseDynamicHtml(
          '<b>Usage:</b><br/>• <code>/alac &lt;apple_music_link | track_id&gt;</code><br/>• Reply to a link with <code>/alac</code><br/>• <code>/alac &lt;link&gt; -f</code> (owner force re-rip)',
        ),
      )
      return
    }

    const isAdmin = auth.isAdmin(msg.sender.id)
    const isForce = parsed.force || isRerip

    if (isForce && !isAdmin) {
      await msg.replyText(
        parseDynamicHtml('This command option is restricted to the bot owner.'),
      )
      return
    }

    const startTime = Date.now()

    // 1. Check Cache (Fast-path)
    if (!isForce) {
      const cached = await service.findCachedTrack(parsed.trackId)
      if (cached) {
        try {
          await tg.sendCopy({
            toChatId: msg.chat.id,
            fromChatId: env.DUMP_CHANNEL_ID,
            message: cached.messageId,
            replyTo: msg.id,
          })

          await service.logRequest({
            telegramId: msg.sender.id,
            chatId: msg.chat.id,
            appleTrackId: parsed.trackId,
            isCacheHit: true,
            durationMs: Date.now() - startTime,
            status: 'completed',
          })
          return
        } catch {
          // If message in dump channel was deleted, proceed to re-rip
        }
      }
    }

    // 2. Slow-path: Queue sequential rip job
    const statusMsg = await msg.replyText(
      parseDynamicHtml('Queued (Position #1)'),
    )

    let lastUpdate = 0
    let lastText = ''

    const updateStatus = async (text: string, force = false) => {
      const now = Date.now()
      if (text === lastText) return
      if (!force && now - lastUpdate < 1500) return
      lastUpdate = now
      lastText = text

      await tg
        .editMessage({
          chatId: msg.chat.id,
          message: statusMsg.id,
          text: parseDynamicHtml(text),
        })
        .catch(() => null)
    }

    try {
      await queue.enqueue(
        async () => {
          await updateStatus('Ripping audio from Apple Music...', true)

          const ripResult = await ripper.rip(parsed.trackId, async (status) => {
            await updateStatus(status)
          })

          await updateStatus('Uploading to Telegram...', true)

          const dumpMsg = await tg.sendMedia(
            env.DUMP_CHANNEL_ID,
            {
              type: 'audio',
              file: Bun.file(ripResult.filePath),
              title: ripResult.title,
              performer: ripResult.artist,
            },
            {
              progressCallback: (uploaded, total) => {
                if (total > 0) {
                  const percent = Math.round((uploaded / total) * 100)
                  updateStatus(`Uploading to Telegram (${percent}%)...`)
                }
              },
            },
          )

          let fileId = ''
          let fileUniqueId: string | undefined
          if (dumpMsg.media && dumpMsg.media.type === 'audio') {
            fileId = dumpMsg.media.fileId
            fileUniqueId = dumpMsg.media.uniqueFileId
          }

          // Save / update in database
          await service.saveTrack({
            appleTrackId: parsed.trackId,
            messageId: dumpMsg.id,
            fileId,
            fileUniqueId,
          })

          // Deliver clean copy to destination chat
          await tg.sendCopy({
            toChatId: msg.chat.id,
            fromChatId: env.DUMP_CHANNEL_ID,
            message: dumpMsg.id,
            replyTo: msg.id,
          })

          // Cleanup local audio file
          try {
            if (existsSync(ripResult.filePath)) {
              unlinkSync(ripResult.filePath)
            }
          } catch {}

          // Delete status message
          await tg
            .deleteMessagesById(msg.chat.id, [statusMsg.id])
            .catch(() => null)

          // Log success
          await service.logRequest({
            telegramId: msg.sender.id,
            chatId: msg.chat.id,
            appleTrackId: parsed.trackId,
            isCacheHit: false,
            durationMs: Date.now() - startTime,
            status: 'completed',
          })
        },
        {
          onPositionChange: (pos) => {
            updateStatus(`Queued (Position #${pos})`, true)
          },
          onStart: () => {
            updateStatus('Ripping audio from Apple Music...', true)
          },
        },
      )
    } catch (err: unknown) {
      const errorMsg =
        err instanceof Error ? err.message : 'Unknown error during ripping'
      await updateStatus(`Failed: ${errorMsg}`, true)

      await service.logRequest({
        telegramId: msg.sender.id,
        chatId: msg.chat.id,
        appleTrackId: parsed.trackId,
        isCacheHit: false,
        durationMs: Date.now() - startTime,
        status: 'failed',
        errorReason: errorMsg,
      })
    }
  })
}
