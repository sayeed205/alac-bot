import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { Message, TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerAuthCommand } from '@/modules/auth/commands/auth.ts'
import { registerListCommand } from '@/modules/auth/commands/list.ts'
import { registerRevokeCommand } from '@/modules/auth/commands/revoke.ts'
import type { CommandContext } from '@/modules/auth/commands/types.ts'
import {
  buildPaginationKeyboard,
  getPeerLink,
  renderAuthListPage,
  resolveTarget,
} from '@/modules/auth/commands/types.ts'
import type { IAuthService } from '@/modules/auth/service.ts'

interface DispatcherInternal {
  _groups: Map<
    number,
    Map<
      string,
      Array<{
        check: (ctx: unknown) => Promise<boolean>
        callback: (ctx: unknown) => Promise<void>
      }>
    >
  >
}

describe('Auth Management Commands & Helpers', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher<TelegramClient>
  let mockAuthService: IAuthService
  let ctx: CommandContext

  beforeEach(() => {
    fakeTg = {
      getChat: mock((peer: string | number) => {
        if (peer === '@durov' || peer === 111) {
          return Promise.resolve({
            id: 111,
            displayName: 'Pavel Durov',
            username: 'durov',
          })
        }
        if (peer === -100987654) {
          return Promise.resolve({
            id: -100987654,
            title: 'Test Supergroup',
          })
        }
        if (peer === -555666) {
          return Promise.resolve({
            id: -555666,
            title: 'Test Basic Group',
          })
        }
        return Promise.reject(new Error('Chat not found'))
      }),
      deleteMessagesById: mock(() => Promise.resolve()),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      sendText: mock(() => Promise.resolve({ id: 1 })),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    dp = Dispatcher.for(fakeTg)

    mockAuthService = {
      isAdmin: mock((id: number) => id === 1),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() =>
        Promise.resolve([
          {
            id: 1,
            telegramId: 111,
            name: 'Pavel Durov',
            createdAt: new Date(),
            updatedAt: new Date(),
          },
        ]),
      ),
    }

    ctx = {
      dp,
      tg: fakeTg,
      service: mockAuthService,
    }
  })

  async function dispatchMessage(
    text: string,
    options: {
      userId?: number
      chatId?: number
      chatType?: 'user' | 'group' | 'supergroup'
      replySender?: {
        id: number
        displayName: string
        type: 'user' | 'chat'
      } | null
    } = {},
  ) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const msg = {
      _name: 'new_message',
      text,
      sender: { id: options.userId ?? 1 },
      chat: {
        id: options.chatId ?? 100,
        type: options.chatType ?? 'user',
        displayName: 'Test Chat',
      },
      getReplyTo: mock(() =>
        Promise.resolve(
          options.replySender ? { sender: options.replySender } : null,
        ),
      ),
      answerText: mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1 })
      }),
      replyText: mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1 })
      }),
    } as unknown as Message

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return { msg, repliedTexts }
  }

  async function dispatchCallback(data: string, userId = 1) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('callback_query') ?? []

    const answeredTexts: string[] = []
    const cb = {
      _name: 'callback_query',
      dataStr: data,
      raw: { data: Buffer.from(data) },
      user: { id: userId },
      chat: { id: 100 },
      messageId: 42,
      answer: mock((options?: { text?: string }) => {
        if (options?.text) answeredTexts.push(options.text)
        return Promise.resolve()
      }),
    }

    for (const h of handlers) {
      if (await h.check(cb)) {
        await h.callback(cb)
      }
    }

    return { cb, answeredTexts }
  }

  describe('Keyboard & Peer Link Helpers', () => {
    it('builds pagination keyboard for single and multiple pages', () => {
      const singlePage = buildPaginationKeyboard(1, 1)
      expect(singlePage).toBeDefined()

      const multiPageMiddle = buildPaginationKeyboard(2, 3)
      expect(multiPageMiddle).toBeDefined()

      const multiPageFirst = buildPaginationKeyboard(1, 3)
      expect(multiPageFirst).toBeDefined()

      const multiPageLast = buildPaginationKeyboard(3, 3)
      expect(multiPageLast).toBeDefined()
    })

    it('generates correct peer links for users and groups', async () => {
      const userWithUsername = await getPeerLink(fakeTg, 111, 'Fallback')
      expect(userWithUsername.link).toBe('https://t.me/durov')
      expect(userWithUsername.name).toBe('Pavel Durov')

      const userWithoutUsername = await getPeerLink(
        fakeTg,
        222,
        'Fallback User',
      )
      expect(userWithoutUsername.link).toBe('tg://user?id=222')

      const supergroup = await getPeerLink(fakeTg, -100987654, 'Supergroup')
      expect(supergroup.link).toContain('https://t.me/c/987654/1')

      const basicGroup = await getPeerLink(fakeTg, -555666, 'Basic Group')
      expect(basicGroup.link).toContain('https://t.me/c/555666/1')
    })

    it('resolves target from reply, arguments, or current group', async () => {
      const replyMsg = {
        text: '/auth',
        chat: { type: 'user', id: 1 },
        getReplyTo: () =>
          Promise.resolve({
            sender: { id: 777, displayName: 'Target User', type: 'user' },
          }),
      } as unknown as Message
      const targetReply = await resolveTarget(replyMsg, fakeTg)
      expect(targetReply?.id).toBe(777)
      expect(targetReply?.isUser).toBe(true)

      const numArgMsg = {
        text: '/auth 111',
        chat: { type: 'user', id: 1 },
        getReplyTo: () => Promise.resolve(null),
      } as unknown as Message
      const targetNum = await resolveTarget(numArgMsg, fakeTg)
      expect(targetNum?.id).toBe(111)

      const userArgMsg = {
        text: '/auth @durov',
        chat: { type: 'user', id: 1 },
        getReplyTo: () => Promise.resolve(null),
      } as unknown as Message
      const targetUser = await resolveTarget(userArgMsg, fakeTg)
      expect(targetUser?.id).toBe(111)

      const unknownMsg = {
        text: '/auth @nonexistent',
        chat: { type: 'user', id: 1 },
        getReplyTo: () => Promise.resolve(null),
      } as unknown as Message
      const targetUnknown = await resolveTarget(unknownMsg, fakeTg)
      expect(targetUnknown).toBeNull()

      const groupMsg = {
        text: '/auth',
        chat: {
          type: 'supergroup',
          id: -100987654,
          displayName: 'Test Supergroup',
        },
        getReplyTo: () => Promise.resolve(null),
      } as unknown as Message
      const targetGroup = await resolveTarget(groupMsg, fakeTg)
      expect(targetGroup?.id).toBe(-100987654)
      expect(targetGroup?.isUser).toBe(false)
    })
  })

  describe('Auth Command', () => {
    it('blocks non-admin users', async () => {
      registerAuthCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/auth 111', {
        userId: 999,
      })
      expect(repliedTexts.length).toBe(0)
    })

    it('shows usage when no target is provided in private chat', async () => {
      registerAuthCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/auth')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Authorization Usage')
    })

    it('authorizes target user successfully', async () => {
      registerAuthCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/auth 111')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Authorized User')
      expect(mockAuthService.authorize).toHaveBeenCalledWith(111, 'Pavel Durov')
    })
  })

  describe('Revoke Command', () => {
    it('blocks non-admin users', async () => {
      registerRevokeCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/revoke 111', {
        userId: 999,
      })
      expect(repliedTexts.length).toBe(0)
    })

    it('shows usage when no target is provided in private chat', async () => {
      registerRevokeCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/revoke')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Revocation Usage')
    })

    it('revokes existing target successfully', async () => {
      registerRevokeCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/revoke 111')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Revoked access for')
    })

    it('handles revoking non-authorized target gracefully', async () => {
      mockAuthService.revoke = mock(() => Promise.resolve({ revoked: false }))
      registerRevokeCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/revoke 111')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Not Found')
    })
  })

  describe('List Command & Callbacks', () => {
    it('renders empty list when no authorizations exist', async () => {
      mockAuthService.listAuthorized = mock(() => Promise.resolve([]))
      await renderAuthListPage(fakeTg, mockAuthService, 100, undefined, 1)
      expect(fakeTg.sendText).toHaveBeenCalled()
    })

    it('renders paginated list and handles callbacks', async () => {
      registerListCommand(ctx)
      await dispatchMessage('/authlist 1')
      expect(fakeTg.sendText).toHaveBeenCalled()

      const nonAdminCb = await dispatchCallback('authpage:1', 999)
      expect(nonAdminCb.answeredTexts).toContain('Unauthorized.')

      await dispatchCallback('noop', 1)

      await dispatchCallback('authclose', 1)
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()

      await dispatchCallback('authpage:2', 1)
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })
  })
})
