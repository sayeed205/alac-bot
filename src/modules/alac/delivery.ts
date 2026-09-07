import type { TelegramClient } from '@mtcute/bun'

import { env } from '@/env.ts'

/**
 * Standard empty caption used when copying media to DM to suppress
 * the dump channel's metadata caption.
 */
export const EMPTY_CAPTION = { text: '' } as const

export interface SendDumpCopyOptions {
  tg: TelegramClient
  toChatId: number | string
  messageId: number
  replyTo?: number
  silent?: boolean
}

/**
 * Helper to copy an audio message from the private dump channel to a target chat
 * with captions stripped, preventing code duplication across command handlers.
 */
export async function sendDumpCopy(options: SendDumpCopyOptions) {
  const { tg, toChatId, messageId, replyTo, silent } = options
  return tg.sendCopy({
    toChatId,
    fromChatId: env.DUMP_CHANNEL_ID,
    message: messageId,
    caption: EMPTY_CAPTION,
    ...(replyTo ? { replyTo } : {}),
    silent,
  })
}
