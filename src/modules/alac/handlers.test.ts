import { describe, expect, it, mock } from 'bun:test'
import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerAlacHandlers } from './handlers.ts'
import type { IAlacService } from './service.ts'
import { registerAuthHandlers } from '@/modules/auth/handlers.ts'
import type { IAuthService } from '@/modules/auth/service.ts'

describe('Handlers Dispatcher & Callback Query Flow', () => {
  it('does not block alac dl: callbacks when auth handlers are registered first', async () => {
    // Mock TG client
    const fakeTg = {
      sendCopy: mock(() => Promise.resolve({ id: 100 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    const dp = Dispatcher.for(fakeTg)

    // Mock services
    const mockAuth: IAuthService = {
      isAdmin: mock(() => true),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
      getAuthorizedCount: mock(() => Promise.resolve(1)),
    }

    const mockAlacService: IAlacService = {
      findCachedTrack: mock((id: string) =>
        Promise.resolve({
          id: 1,
          appleTrackId: id,
          messageId: 42,
          fileId: 'fid',
          title: 'Test Song',
          artist: 'Test Artist',
          album: 'Test Album',
          duration: 200,
          bitDepth: 16,
          sampleRate: 44100,
          createdAt: new Date(),
          updatedAt: new Date(),
        }),
      ),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve({} as any)),
      searchCachedTracks: mock(() => Promise.resolve([])),
      logRequest: mock(() => Promise.resolve()),
      getStats: mock(() => Promise.resolve({} as any)),
      deleteTrack: mock(() => Promise.resolve(true)),
    }

    // Register handlers in standard order: auth first, alac second
    registerAuthHandlers(dp, fakeTg, mockAuth)
    registerAlacHandlers(
      dp,
      fakeTg,
      mockAlacService,
      undefined as any,
      undefined as any,
      mockAuth,
    )

    // Extract callback_query handlers from group 0
    const group0 = (dp as any)._groups.get(0)
    const cbHandlers = group0.get('callback_query')
    expect(cbHandlers.length).toBe(2)

    const [authCb, alacCb] = cbHandlers

    // 1. Simulate a dl: callback query context
    let dlAnswerText = ''
    const dlQueryCtx = {
      _name: 'callback_query',
      raw: { data: new Uint8Array([1]) },
      dataStr: 'dl:1559523359',
      user: { id: 12345 },
      chat: { id: 67890 },
      messageId: 99,
      answer: mock((opts?: { text?: string }) => {
        dlAnswerText = opts?.text || ''
        return Promise.resolve()
      }),
    }

    // auth check should reject dl:
    const authMatchedDl = await authCb.check(dlQueryCtx)
    expect(authMatchedDl).toBeFalsy()

    // alac check should match dl:
    const alacMatchedDl = await alacCb.check(dlQueryCtx)
    expect(alacMatchedDl).toBeTruthy()

    // Run alac callback handler
    await alacCb.callback(dlQueryCtx)

    // Verify callback was answered and sendCopy was called
    expect(dlQueryCtx.answer).toHaveBeenCalled()
    expect(fakeTg.sendCopy).toHaveBeenCalledWith({
      toChatId: 67890,
      fromChatId: expect.anything(),
      message: 42,
      replyTo: 99,
    })
    expect(mockAlacService.findCachedTrack).toHaveBeenCalledWith('1559523359')

    // 2. Simulate search_close callback
    const closeQueryCtx = {
      _name: 'callback_query',
      raw: { data: new Uint8Array([1]) },
      dataStr: 'search_close',
      user: { id: 12345 },
      chat: { id: 67890 },
      messageId: 99,
      answer: mock(() => Promise.resolve()),
    }

    const authMatchedClose = await authCb.check(closeQueryCtx)
    expect(authMatchedClose).toBeFalsy()

    const alacMatchedClose = await alacCb.check(closeQueryCtx)
    expect(alacMatchedClose).toBeTruthy()

    await alacCb.callback(closeQueryCtx)
    expect(closeQueryCtx.answer).toHaveBeenCalled()
    expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(67890, [99])

    // 3. Simulate authpage: callback
    const authPageCtx = {
      _name: 'callback_query',
      raw: { data: new Uint8Array([1]) },
      dataStr: 'authpage:2',
      user: { id: 12345 },
      chat: { id: 67890 },
      messageId: 99,
      answer: mock(() => Promise.resolve()),
    }

    const authMatchedPage = await authCb.check(authPageCtx)
    expect(authMatchedPage).toBeTruthy()

    const alacMatchedPage = await alacCb.check(authPageCtx)
    expect(alacMatchedPage).toBeFalsy()
  })
})
