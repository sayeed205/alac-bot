import {
  BotKeyboard,
  html,
  type Message,
  type TelegramClient,
} from '@mtcute/bun'
import { type Dispatcher, filters } from '@mtcute/dispatcher'

import { authService, type IAuthService } from '@/modules/auth/service.ts'

interface TargetPeer {
  id: number
  name: string
  isUser: boolean
}

const PAGE_SIZE = 20

function parseDynamicHtml(content: string) {
  return html([content] as unknown as TemplateStringsArray)
}

function buildPaginationKeyboard(page: number, totalPages: number) {
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

async function getPeerLink(
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

  // Supergroups and channels start with -100
  const strId = String(id)
  if (chat && 'username' in chat && chat.username) {
    return { link: `https://t.me/${chat.username}`, name }
  }

  if (strId.startsWith('-100')) {
    const bareId = strId.slice(4)
    return { link: `https://t.me/c/${bareId}/1`, name }
  }

  // Basic group
  const bareId = strId.replace(/^-/, '')
  return { link: `https://t.me/c/${bareId}/1`, name }
}

async function renderAuthListPage(
  tg: TelegramClient,
  service: IAuthService,
  chatId: number,
  messageId: number | undefined,
  requestedPage: number,
  replyToMessageId?: number,
) {
  const list = await service.listAuthorized()
  if (list.length === 0) {
    const text = 'No users or groups authorized yet.'
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
  const totalPages = Math.max(1, Math.ceil(totalItems / PAGE_SIZE))
  const currentPage = Math.min(Math.max(1, requestedPage), totalPages)
  const startIndex = (currentPage - 1) * PAGE_SIZE
  const pageItems = list.slice(startIndex, startIndex + PAGE_SIZE)

  const rows = await Promise.all(
    pageItems.map(async (item, index) => {
      const { link, name } = await getPeerLink(
        tg,
        item.telegramId,
        item.name || 'Unknown',
      )
      const sanitizedName = name.replace(/\|/g, '\\|').replace(/[[\]]/g, '')
      const globalIndex = startIndex + index + 1
      return `| ${globalIndex} | [${sanitizedName}](${link}) | \`${item.telegramId}\` |`
    }),
  )

  const header =
    totalPages > 1
      ? `# Authorized List (${currentPage}/${totalPages}, Total: ${totalItems})`
      : `# Authorized List (${totalItems})`

  const markdown = `${header}\n\n| # | Target | Telegram ID |\n|---|---|---|\n${rows.join('\n')}`
  const keyboard = buildPaginationKeyboard(currentPage, totalPages)

  if (messageId) {
    try {
      await tg.editMessage({
        chatId,
        message: messageId,
        richMessage: {
          type: 'markdown',
          content: markdown,
        },
        replyMarkup: keyboard,
      })
    } catch (err: unknown) {
      if (
        err &&
        typeof err === 'object' &&
        'message' in err &&
        String((err as { message: unknown }).message).includes('NOT_MODIFIED')
      ) {
        return
      }

      // If rich edit fails, fallback to HTML edit
      const fallbackRows = await Promise.all(
        pageItems.map(async (item, index) => {
          const { link, name } = await getPeerLink(
            tg,
            item.telegramId,
            item.name || 'Unknown',
          )
          const escapedName = html.escape(name)
          const globalIndex = startIndex + index + 1
          return `${globalIndex}. <a href="${link}">${escapedName}</a> — <code>${item.telegramId}</code>`
        }),
      )
      const body = `<b>${header.replace('# ', '')}:</b><br/><br/>${fallbackRows.join('<br/>')}`
      await tg.editMessage({
        chatId,
        message: messageId,
        text: parseDynamicHtml(body),
        replyMarkup: keyboard,
      })
    }
  } else {
    try {
      await tg.sendRichMessage(chatId, {
        content: {
          type: 'markdown',
          content: markdown,
        },
        replyMarkup: keyboard,
        replyTo: replyToMessageId,
      })
    } catch {
      // Fallback to standard message
      const fallbackRows = await Promise.all(
        pageItems.map(async (item, index) => {
          const { link, name } = await getPeerLink(
            tg,
            item.telegramId,
            item.name || 'Unknown',
          )
          const escapedName = html.escape(name)
          const globalIndex = startIndex + index + 1
          return `${globalIndex}. <a href="${link}">${escapedName}</a> — <code>${item.telegramId}</code>`
        }),
      )
      const body = `<b>${header.replace('# ', '')}:</b><br/><br/>${fallbackRows.join('<br/>')}`
      await tg.sendText(chatId, parseDynamicHtml(body), {
        replyMarkup: keyboard,
        replyTo: replyToMessageId,
      })
    }
  }
}

async function resolveTarget(
  msg: Message,
  tg: TelegramClient,
): Promise<TargetPeer | null> {
  // 1. Target by reply
  if (msg.replyToMessage?.sender) {
    const sender = msg.replyToMessage.sender
    if (sender.type !== 'anonymous') {
      const username = sender.type === 'user' ? sender.username : null
      const name =
        sender.displayName || (username ? `@${username}` : `Peer ${sender.id}`)
      return {
        id: sender.id,
        name,
        isUser: sender.type === 'user',
      }
    }
  }

  // 2. Target by explicit argument (e.g. /auth 123456 or /auth @username)
  const textParts = msg.text.trim().split(/\s+/)
  const arg = textParts[1]?.trim()
  if (arg) {
    if (/^-?\d+$/.test(arg)) {
      const id = Number(arg)
      const chat = await tg.getChat(id).catch(() => null)
      return {
        id,
        name: chat?.displayName || chat?.title || String(id),
        isUser: id > 0,
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

  // 3. Target current group/channel if invoked without args inside a group
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

export function registerAuthHandlers(
  dp: Dispatcher<TelegramClient>,
  tg: TelegramClient,
  service: IAuthService = authService,
) {
  // Command: /auth
  dp.onNewMessage(filters.command('auth'), async (msg) => {
    if (!service.isAdmin(msg.sender.id)) {
      return
    }

    const target = await resolveTarget(msg, tg)
    if (!target) {
      await msg.answerText(
        parseDynamicHtml(
          '<b>Usage:</b><br/>• Reply to a message with <code>/auth</code><br/>• <code>/auth &lt;id | @username&gt;</code><br/>• Send <code>/auth</code> inside a group to authorize the group',
        ),
      )
      return
    }

    const { link, name } = await getPeerLink(tg, target.id, target.name)
    const { newlyAdded } = await service.authorize(target.id, name)
    const escapedName = html.escape(name)
    const typeLabel = target.isUser ? '' : 'group '

    await msg.answerText(
      parseDynamicHtml(
        `${newlyAdded ? 'Authorized' : 'Updated'} ${typeLabel}<a href="${link}">${escapedName}</a> (<code>${target.id}</code>)`,
      ),
    )
  })

  // Command: /revoke & /unauth
  dp.onNewMessage(filters.command(['revoke', 'unauth']), async (msg) => {
    if (!service.isAdmin(msg.sender.id)) {
      return
    }

    const target = await resolveTarget(msg, tg)
    if (!target) {
      await msg.answerText(
        parseDynamicHtml(
          '<b>Usage:</b><br/>• Reply to a message with <code>/revoke</code><br/>• <code>/revoke &lt;id | @username&gt;</code><br/>• Send <code>/revoke</code> inside a group to revoke the group',
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
          `Revoked access for <a href="${link}">${escapedName}</a> (<code>${target.id}</code>)`,
        ),
      )
    } else {
      await msg.answerText(
        parseDynamicHtml(
          `ID <code>${target.id}</code> was not found in authorized list.`,
        ),
      )
    }
  })

  // Command: /authlist [page]
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

  // Callback query for pagination & actions
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
  })
}
