import { BotKeyboard, html, type TelegramClient } from '@mtcute/bun'
import type { Dispatcher, MessageContext } from '@mtcute/dispatcher'

import type { IAuthService } from '@/modules/auth/service.ts'

export interface TargetPeer {
  id: number
  name: string
  isUser: boolean
}

export interface CommandContext {
  dp: Dispatcher
  tg: TelegramClient
  service: IAuthService
}

export const PAGE_SIZE = 20

export function parseDynamicHtml(content: string) {
  return html([content] as unknown as TemplateStringsArray)
}

export function buildPaginationKeyboard(page: number, totalPages: number) {
  if (totalPages <= 1) {
    return BotKeyboard.inline([
      [
        BotKeyboard.callback('Refresh', `authpage:${page}`),
        BotKeyboard.callback('Close', 'authclose'),
      ],
    ])
  }

  const navRow = []
  if (page > 1) {
    navRow.push(BotKeyboard.callback('< Prev', `authpage:${page - 1}`))
  }
  navRow.push(BotKeyboard.callback(`${page} / ${totalPages}`, 'noop'))
  if (page < totalPages) {
    navRow.push(BotKeyboard.callback('Next >', `authpage:${page + 1}`))
  }

  return BotKeyboard.inline([
    navRow,
    [
      BotKeyboard.callback('Refresh', `authpage:${page}`),
      BotKeyboard.callback('Close', 'authclose'),
    ],
  ])
}

export async function getPeerLink(
  tg: TelegramClient,
  id: number,
  fallbackName: string,
): Promise<{ link: string; name: string }> {
  const chat = await tg.getChat(id).catch(() => null)
  const name = chat?.displayName || chat?.title || fallbackName

  if (id > 0) {
    if (chat && 'username' in chat && chat.username) {
      return { link: `https://t.me/${chat.username}`, name }
    }
    return { link: `tg://user?id=${id}`, name }
  }

  const strId = String(id)
  if (chat && 'username' in chat && chat.username) {
    return { link: `https://t.me/${chat.username}`, name }
  }

  if (strId.startsWith('-100')) {
    const bareId = strId.slice(4)
    return { link: `https://t.me/c/${bareId}/1`, name }
  }

  const bareId = strId.replace(/^-/, '')
  return { link: `https://t.me/c/${bareId}/1`, name }
}

export async function resolveTarget(
  msg: MessageContext,
  tg: TelegramClient,
): Promise<TargetPeer | null> {
  const reply = await msg.getReplyTo().catch(() => null)
  if (reply?.sender) {
    const sender = reply.sender
    const name = sender.displayName || sender.username || `User ${sender.id}`
    return {
      id: sender.id,
      name,
      isUser: sender.type === 'user',
    }
  }

  const textParts = msg.text.trim().split(/\s+/)
  const arg = textParts[1]?.trim()
  if (arg) {
    const numId = Number.parseInt(arg, 10)
    if (!Number.isNaN(numId)) {
      const chat = await tg.getChat(numId).catch(() => null)
      const name = chat?.displayName || chat?.title || `User ${numId}`
      return {
        id: numId,
        name,
        isUser: numId > 0,
      }
    }

    const chat = await tg.getChat(arg).catch(() => null)
    if (chat) {
      return {
        id: chat.id,
        name: chat.displayName || chat.title || arg,
        isUser: chat.id > 0,
      }
    }

    return null
  }

  if (msg.chat.type !== 'user') {
    const chatId = msg.chat.id
    return {
      id: chatId,
      name: msg.chat.displayName || `Chat ${chatId}`,
      isUser: false,
    }
  }

  return null
}

export async function renderAuthListPage(
  tg: TelegramClient,
  service: IAuthService,
  chatId: number,
  messageId: number | undefined,
  requestedPage: number,
  replyToMessageId?: number,
) {
  const list = await service.listAuthorized()
  if (list.length === 0) {
    const text = parseDynamicHtml(
      'ℹ️ <b>No users or groups authorized yet.</b><br/>' +
        'Use <code>/auth &lt;id | @username&gt;</code> to grant access.',
    )
    if (messageId) {
      await tg
        .editMessage({
          chatId,
          message: messageId,
          text,
        })
        .catch(() => null)
    } else {
      await tg.sendText(chatId, text, { replyTo: replyToMessageId })
    }
    return
  }

  const totalItems = list.length
  const totalPages = Math.ceil(totalItems / PAGE_SIZE)
  const currentPage = Math.max(1, Math.min(requestedPage, totalPages))
  const startIndex = (currentPage - 1) * PAGE_SIZE
  const pageItems = list.slice(startIndex, startIndex + PAGE_SIZE)

  const rows = await Promise.all(
    pageItems.map(async (item, index) => {
      const { link, name } = await getPeerLink(
        tg,
        item.telegramId,
        item.name || 'Unknown',
      )
      const escapedName = html.escape(name)
      const globalIndex = startIndex + index + 1
      const isGroup = item.telegramId < 0
      const icon = isGroup ? '👥' : '👤'
      return `${globalIndex}. ${icon} <a href="${link}">${escapedName}</a> — <code>${item.telegramId}</code>`
    }),
  )

  const pageInfo =
    totalPages > 1
      ? `<i>Page ${currentPage}/${totalPages} • Total: ${totalItems}</i>`
      : `<i>Total: ${totalItems}</i>`

  const body =
    `👥 <b>Authorized Users & Groups</b><br/>${pageInfo}<br/><br/>` +
    `<blockquote>${rows.join('<br/>')}</blockquote>`

  const keyboard = buildPaginationKeyboard(currentPage, totalPages)

  if (messageId) {
    await tg
      .editMessage({
        chatId,
        message: messageId,
        text: parseDynamicHtml(body),
        replyMarkup: keyboard,
      })
      .catch((err: unknown) => {
        if (
          err &&
          typeof err === 'object' &&
          'message' in err &&
          String((err as { message: unknown }).message).includes('NOT_MODIFIED')
        ) {
          return
        }
      })
  } else {
    await tg.sendText(chatId, parseDynamicHtml(body), {
      replyMarkup: keyboard,
      replyTo: replyToMessageId,
    })
  }
}
