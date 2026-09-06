import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerAlacCommands } from '@/modules/alac/commands/index.ts'
import type { IRipQueue } from '@/modules/alac/queue.ts'
import type { ITrackRipper } from '@/modules/alac/ripper.ts'
import type { IAlacService } from '@/modules/alac/service.ts'
import { registerAuthCommands } from '@/modules/auth/commands/index.ts'
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

describe('Handlers Dispatcher & Callback Query Flow', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService

  beforeEach(() => {
    fakeTg = {
      sendText: mock(() => Promise.resolve({ id: 1 })),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    dp = Dispatcher.for(fakeTg)

    mockAuth = {
      isAdmin: mock((id: number) => id === 1),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }
  })

  it('does not block alac dl: callbacks when auth handlers are registered first', async () => {
    const mockAlacService: IAlacService = {
      findCachedTrack: mock(() =>
        Promise.resolve({
          id: 1,
          appleTrackId: '1559523359',
          messageId: 42,
          fileId: 'fid',
          fileUniqueId: 'uid',
          title: 'Title',
          artist: 'Artist',
          album: 'Album',
          duration: 200,
          bitDepth: 24,
          sampleRate: 48000,
          genre: 'Pop',
          releaseDate: '2021',
          trackNumber: 1,
          trackCount: 1,
          createdAt: new Date(),
          updatedAt: new Date(),
        }),
      ),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve({} as never)),
      searchCachedTracks: mock(() => Promise.resolve([])),
      logRequest: mock(() => Promise.resolve()),
      getStats: mock(() => Promise.resolve({} as never)),
      deleteTrack: mock(() => Promise.resolve(true)),
      getAllTrackIds: mock(() => Promise.resolve([])),
      deleteTracksNotIn: mock(() => Promise.resolve(0)),
    }

    registerAuthCommands(dp, fakeTg, mockAuth)
    registerAlacCommands(
      dp,
      fakeTg,
      mockAlacService,
      undefined as unknown as ITrackRipper,
      undefined as unknown as IRipQueue,
      mockAuth,
    )

    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const cbHandlers = group0?.get('callback_query') ?? []
    expect(cbHandlers.length).toBe(4)

    const authCb = cbHandlers[0]
    const searchCb = cbHandlers[1]
    const cancelCb = cbHandlers[2]
    if (!authCb || !searchCb || !cancelCb) {
      throw new Error('Expected 3 callback handlers registered')
    }

    const dlQueryCtx = {
      _name: 'callback_query',
      raw: { data: new Uint8Array([1]) },
      dataStr: 'dl:1559523359',
      user: { id: 12345 },
      chat: { id: 67890 },
      messageId: 99,
      answer: mock(() => Promise.resolve()),
    }

    const authMatchedDl = await authCb.check(dlQueryCtx)
    expect(authMatchedDl).toBeFalsy()

    const searchMatchedDl = await searchCb.check(dlQueryCtx)
    expect(searchMatchedDl).toBeTruthy()

    const cancelMatchedDl = await cancelCb.check(dlQueryCtx)
    expect(cancelMatchedDl).toBeFalsy()
  })
})
