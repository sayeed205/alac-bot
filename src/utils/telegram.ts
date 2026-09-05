import type {
  InputPeerLike,
  InputText,
  ReplyMarkup,
  TelegramClient,
} from '@mtcute/bun'

import { error, warn } from './logger.ts'

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

export function extractFloodWaitSeconds(err: unknown): number | null {
  if (!err) return null
  const errStr =
    (err as { text?: string })?.text || (err as Error)?.message || String(err)
  const match = errStr.match(
    /(?:FLOOD_WAIT_|FLOOD_PREMIUM_WAIT_|SLOWMODE_WAIT_)(\d+)/,
  )
  if (match?.[1]) {
    const sec = parseInt(match[1], 10)
    return Number.isNaN(sec) ? null : sec
  }
  return null
}

export interface EditMessageOptions {
  chatId: InputPeerLike
  message: number
  text?: InputText
  replyMarkup?: ReplyMarkup
  block?: boolean
  maxWaitSec?: number
}

export async function editMessageSafe(
  tg: TelegramClient,
  opts: EditMessageOptions,
): Promise<boolean> {
  const {
    chatId,
    message,
    text,
    replyMarkup,
    block = true,
    maxWaitSec = 60,
  } = opts

  try {
    await tg.editMessage({
      chatId,
      message,
      text,
      replyMarkup,
    })
    return true
  } catch (err: unknown) {
    const floodSec = extractFloodWaitSeconds(err)
    if (floodSec !== null) {
      warn(`Telegram editMessage FloodWait: ${floodSec}s`, {
        chatId: String(chatId),
        messageId: message,
        waitSeconds: floodSec,
        block,
      })
      if (!block) {
        return false
      }
      if (floodSec <= maxWaitSec) {
        const waitMs = Math.ceil(floodSec * 1.2 * 1000)
        await sleep(waitMs)
        return editMessageSafe(tg, {
          ...opts,
          maxWaitSec: maxWaitSec - floodSec,
        })
      }
      return false
    }

    const errStr = String(err)
    if (errStr.includes('MESSAGE_NOT_MODIFIED')) {
      return true
    }

    error('Failed to edit Telegram message', {
      chatId: String(chatId),
      messageId: message,
      error: errStr,
    })
    return false
  }
}

export interface SendTextOptions {
  chatId: Parameters<TelegramClient['sendText']>[0]
  text: Parameters<TelegramClient['sendText']>[1]
  params?: Parameters<TelegramClient['sendText']>[2]
  block?: boolean
  maxWaitSec?: number
}

export async function sendTextSafe(
  tg: TelegramClient,
  opts: SendTextOptions,
): Promise<boolean> {
  const { chatId, text, params, block = true, maxWaitSec = 60 } = opts

  try {
    await tg.sendText(chatId, text, params)
    return true
  } catch (err: unknown) {
    const floodSec = extractFloodWaitSeconds(err)
    if (floodSec !== null) {
      warn(`Telegram sendText FloodWait: ${floodSec}s`, {
        chatId: String(chatId),
        waitSeconds: floodSec,
        block,
      })
      if (!block) {
        return false
      }
      if (floodSec <= maxWaitSec) {
        const waitMs = Math.ceil(floodSec * 1.2 * 1000)
        await sleep(waitMs)
        return sendTextSafe(tg, {
          ...opts,
          maxWaitSec: maxWaitSec - floodSec,
        })
      }
      return false
    }

    error('Failed to send Telegram text', {
      chatId: String(chatId),
      error: String(err),
    })
    return false
  }
}
