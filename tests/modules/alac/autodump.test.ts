import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'
import type { Dispatcher, MessageContext } from '@mtcute/dispatcher'

import {
  discoverNewTracks,
  getCutoffDateString,
  isAutoDumpInProgress,
  registerAutoDumpCommand,
  resetAutoDumpState,
  startAutoDumpScheduler,
} from '@/modules/alac/commands/autodump.ts'
import type { CommandContext } from '@/modules/alac/commands/types.ts'
import type { IAlacService } from '@/modules/alac/service.ts'
import type { IAuthService } from '@/modules/auth/service.ts'
import type { ISettingsService } from '@/modules/settings/service.ts'

describe('AutoDump Module', () => {
  const ADMIN_ID = 99999
  const REGULAR_ID = 11111

  let mockTg: TelegramClient
  let mockAuth: IAuthService
  let mockSettings: ISettingsService
  let mockService: IAlacService
  let commandHandlers: Map<string, (msg: MessageContext) => Promise<void>>

  beforeEach(() => {
    process.env.ADMIN_ID = String(ADMIN_ID)
    resetAutoDumpState()
    commandHandlers = new Map()

    mockTg = {
      sendText: mock(() => Promise.resolve({ id: 101 })),
      editMessage: mock(() => Promise.resolve({ id: 101 })),
      deleteMessagesById: mock(() => Promise.resolve()),
    } as unknown as TelegramClient

    mockAuth = {
      isAdmin: (id: number) => id === ADMIN_ID,
      isAuthorized: () => Promise.resolve(true),
      authorize: () => Promise.resolve({ newlyAdded: true }),
      revoke: () => Promise.resolve({ revoked: true }),
      listAuthorized: () => Promise.resolve([]),
    }

    mockSettings = {
      isAutoDumpEnabled: mock(() => true),
      getAutoDumpStorefronts: mock(() => ['us']),
    } as unknown as ISettingsService

    mockService = {
      findCachedTrack: mock(() => Promise.resolve(null)),
      findTrackByFileUniqueId: mock(() => Promise.resolve(null)),
      saveTrack: mock(() => Promise.resolve({} as never)),
      logRequest: mock(() => Promise.resolve()),
    } as unknown as IAlacService
  })

  function createMockContext(): CommandContext {
    const dp = {
      onNewMessage: mock(
        (_filter: unknown, handler: (msg: MessageContext) => Promise<void>) => {
          commandHandlers.set('message', handler)
        },
      ),
    } as unknown as Dispatcher

    return {
      dp,
      tg: mockTg,
      service: mockService,
      ripper: {} as never,
      queue: {
        enqueue: mock((task) => task(new AbortController().signal)),
      } as never,
      auth: mockAuth,
      settings: mockSettings,
    }
  }

  it('calculates correct cutoff date string for X days', () => {
    const cutoff1 = getCutoffDateString(1)
    const cutoff3 = getCutoffDateString(3)

    expect(cutoff1).toMatch(/^\d{4}-\d{2}-\d{2}$/)
    expect(cutoff3).toMatch(/^\d{4}-\d{2}-\d{2}$/)
    expect(new Date(cutoff1).getTime()).toBeGreaterThan(
      new Date(cutoff3).getTime(),
    )
  })

  it('discovers tracks using mock fetch matching cutoff date', async () => {
    const originalFetch = globalThis.fetch
    const mockToday = new Date().toISOString().slice(0, 10)

    globalThis.fetch = mock(async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('playlists/pl.2b0e6e332fdf4b7a91164da3162127b5')) {
        return new Response(
          JSON.stringify({
            data: [
              {
                relationships: {
                  tracks: {
                    data: [
                      {
                        id: '1234567890',
                        attributes: { releaseDate: mockToday },
                      },
                      {
                        id: '9999999999',
                        attributes: { releaseDate: '2020-01-01' },
                      },
                    ],
                  },
                },
              },
            ],
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }

      if (urlStr.includes('albums.json')) {
        return new Response(
          JSON.stringify({
            feed: {
              results: [
                { id: 'album1', releaseDate: mockToday },
                { id: 'album_old', releaseDate: '2019-01-01' },
              ],
            },
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }

      if (urlStr.includes('lookup?id=album1')) {
        return new Response(
          JSON.stringify({
            results: [
              { wrapperType: 'collection' },
              { wrapperType: 'track', trackId: 22334455 },
            ],
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }

      return new Response('{}', { status: 404 })
    }) as unknown as typeof fetch

    try {
      const result = await discoverNewTracks(['us'], 1)
      expect(result.trackIds).toContain('1234567890')
      expect(result.trackIds).toContain('22334455')
      expect(result.trackIds).not.toContain('9999999999')
      expect(result.totalFound).toBe(2)
      expect(result.storefrontCounts.us).toBe(2)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  it('blocks non-admin users from running /dumpnew', async () => {
    const ctx = createMockContext()
    registerAutoDumpCommand(ctx)

    const handler = commandHandlers.get('message')
    expect(handler).toBeDefined()

    const replyTextMock = mock(() => Promise.resolve({ id: 102 }))
    const msg = {
      sender: { id: REGULAR_ID },
      chat: { id: REGULAR_ID },
      text: '/dumpnew 3',
      replyText: replyTextMock,
    } as unknown as MessageContext

    if (handler) {
      await handler(msg)
    }
    expect(replyTextMock).not.toHaveBeenCalled()
    expect(mockTg.sendText).not.toHaveBeenCalled()
  })

  it('runs /dumpnew for admin and rejects concurrent sweeps', async () => {
    const ctx = createMockContext()
    registerAutoDumpCommand(ctx)

    const handler = commandHandlers.get('message')
    const replyTextMock = mock(() => Promise.resolve({ id: 102 }))

    const msg = {
      sender: { id: ADMIN_ID, displayName: 'Admin' },
      chat: { id: ADMIN_ID },
      text: '/dumpnew 1',
      replyText: replyTextMock,
    } as unknown as MessageContext

    // Mock discover to return empty
    const originalFetch = globalThis.fetch
    globalThis.fetch = mock(
      async () => new Response('{}', { status: 404 }),
    ) as unknown as typeof fetch

    try {
      if (!handler) throw new Error('Handler not defined')
      const p = handler(msg)
      expect(isAutoDumpInProgress()).toBe(true)

      // Concurrent call while in progress
      const replyBusyMock = mock(() => Promise.resolve({ id: 103 }))
      const busyMsg = {
        sender: { id: ADMIN_ID },
        chat: { id: ADMIN_ID },
        text: '/dumpnew 1',
        replyText: replyBusyMock,
      } as unknown as MessageContext
      await handler(busyMsg)
      expect(replyBusyMock).toHaveBeenCalled()

      await p
      expect(isAutoDumpInProgress()).toBe(false)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  it('startAutoDumpScheduler creates and clears interval', () => {
    const ctx = createMockContext()
    const stop = startAutoDumpScheduler(ctx)
    expect(typeof stop).toBe('function')
    stop()
  })
})
