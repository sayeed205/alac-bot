import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerCleanCommand } from '@/modules/alac/commands/clean.ts'
import { registerDeleteCommand } from '@/modules/alac/commands/delete.ts'
import { registerHealthCommand } from '@/modules/alac/commands/health.ts'
import { registerHelpCommand } from '@/modules/alac/commands/help.ts'
import { registerIndexCommand } from '@/modules/alac/commands/index_cmd.ts'
import { registerInfoCommand } from '@/modules/alac/commands/info.ts'
import { registerQueueCommand } from '@/modules/alac/commands/queue.ts'
import { registerSearchCommand } from '@/modules/alac/commands/search.ts'
import { registerStatsCommand } from '@/modules/alac/commands/stats.ts'
import type { CommandContext } from '@/modules/alac/commands/types.ts'
import type { IRipQueue } from '@/modules/alac/queue.ts'
import type { ITrackRipper } from '@/modules/alac/ripper.ts'
import type { IAlacService } from '@/modules/alac/service.ts'
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

describe('ALAC Management Commands', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockService: IAlacService
  let mockQueue: IRipQueue
  let mockRipper: ITrackRipper
  let ctx: CommandContext

  beforeEach(() => {
    fakeTg = {
      sendCopy: mock(() => Promise.resolve({ id: 100 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      sendText: mock(() => Promise.resolve({ id: 101 })),
      sendMedia: mock(() =>
        Promise.resolve({
          id: 102,
          media: { type: 'audio', fileId: 'fid', uniqueFileId: 'uid' },
        }),
      ),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    dp = Dispatcher.for(fakeTg)

    mockAuth = {
      isAdmin: mock(() => true),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    mockService = {
      findCachedTrack: mock((id: string) => {
        if (id === '12345') {
          return Promise.resolve({
            id: 1,
            appleTrackId: '12345',
            messageId: 42,
            fileId: 'fid',
            fileUniqueId: 'uid',
            title: 'Test Song',
            artist: 'Test Artist',
            album: 'Test Album',
            duration: 200,
            bitDepth: 24,
            sampleRate: 48000,
            genre: 'Pop',
            releaseDate: '2023-01-01',
            trackNumber: 1,
            trackCount: 10,
            createdAt: new Date(),
            updatedAt: new Date(),
          })
        }
        return Promise.resolve(null)
      }),
      findTrackByFileUniqueId: mock(() => Promise.resolve(null)),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve({} as never)),
      searchCachedTracks: mock(() => Promise.resolve([])),
      logRequest: mock(() => Promise.resolve()),
      getStats: mock(() =>
        Promise.resolve({
          totalCachedTracks: 10,
          totalRequests: 20,
          cacheHits: 15,
          cacheMisses: 5,
          totalFailedRequests: 0,
          cacheHitRatio: 75.0,
          avgCacheDurationMs: 50,
          avgRipDurationMs: 2000,
          topTracks: [],
        }),
      ),
      deleteTrack: mock(() => Promise.resolve(true)),
      getAllTrackIds: mock(() => Promise.resolve([])),
      deleteTracksNotIn: mock(() => Promise.resolve(0)),
    }

    mockQueue = {
      enqueue: mock((task) =>
        task(new AbortController().signal),
      ) as unknown as IRipQueue['enqueue'],
      getPendingCount: mock(() => 0),
      isProcessing: mock(() => false),
      clear: mock(() => {}),
    }

    mockRipper = {
      rip: mock(() =>
        Promise.resolve({
          filePath: '/tmp/test_track_rip.m4a',
          title: 'Test Song',
          artist: 'Test Artist',
          album: 'Test Album',
          duration: 200,
          bitDepth: 24,
          sampleRate: 48000,
          codec: 'alac',
          genre: 'Pop',
          releaseDate: '2021-01-01',
          trackNumber: 1,
          trackCount: 1,
        }),
      ),
    }

    ctx = {
      dp,
      tg: fakeTg,
      service: mockService,
      queue: mockQueue,
      ripper: mockRipper,
      auth: mockAuth,
    }
  })

  async function dispatchMessage(text: string, isAdminUser = true) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const msg = {
      _name: 'new_message',
      text,
      sender: { id: isAdminUser ? 1 : 999 },
      chat: { id: 100 },
      getReplyTo: mock(() => Promise.resolve(null)),
      replyText: mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1, chat: { id: 100 } })
      }),
    }

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

  describe('Queue Command', () => {
    it('reports idle when no tasks are queued', async () => {
      registerQueueCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/queue')

      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Rip Queue is Idle')
    })

    it('reports active processing when tasks are in queue', async () => {
      mockQueue.isProcessing = mock(() => true)
      mockQueue.getPendingCount = mock(() => 2)

      registerQueueCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/queue')

      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Rip Queue Status')
      expect(repliedTexts[0]).toContain('Active')
      expect(repliedTexts[0]).toContain('2')
    })
  })

  describe('Delete Command', () => {
    it('blocks non-admin users', async () => {
      mockAuth.isAdmin = mock(() => false)
      registerDeleteCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/delete 12345', false)

      expect(repliedTexts[0]).toContain('Access Restricted')
      expect(mockService.deleteTrack).not.toHaveBeenCalled()
    })

    it('deletes cached track from database and dump channel', async () => {
      registerDeleteCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/delete 12345')

      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
      expect(mockService.deleteTrack).toHaveBeenCalledWith('12345')
      expect(repliedTexts[0]).toContain('Track Deleted Successfully')
    })

    it('handles non-cached track gracefully', async () => {
      registerDeleteCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/delete 99999')

      expect(repliedTexts[0]).toContain('Track Not Found')
      expect(mockService.deleteTrack).not.toHaveBeenCalled()
    })
  })

  describe('Clean Command', () => {
    const testDir = path.resolve('bot-data/downloads')

    beforeEach(() => {
      fs.mkdirSync(testDir, { recursive: true })
      fs.writeFileSync(path.join(testDir, 'temp_test.m4a'), 'dummy content')
    })

    afterEach(() => {
      try {
        const file = path.join(testDir, 'temp_test.m4a')
        if (fs.existsSync(file)) fs.unlinkSync(file)
      } catch {}
    })

    it('blocks non-admin users', async () => {
      mockAuth.isAdmin = mock(() => false)
      registerCleanCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/clean', false)

      expect(repliedTexts[0]).toContain('Access Restricted')
    })

    it('removes leftover files and reports space freed', async () => {
      registerCleanCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/clean')

      expect(repliedTexts[0]).toContain('Temporary Storage Cleaned')
      expect(repliedTexts[0]).toContain('Files Removed')
      expect(fs.existsSync(path.join(testDir, 'temp_test.m4a'))).toBe(false)
    })
  })

  describe('Info Command', () => {
    it('returns usage guide when no arguments or reply provided', async () => {
      registerInfoCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/info')

      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Track Info Usage')
    })

    it('displays track information and cache status when track is cached', async () => {
      const originalFetch = globalThis.fetch
      globalThis.fetch = mock(() =>
        Promise.resolve({
          ok: true,
          json: () =>
            Promise.resolve({
              resultCount: 1,
              results: [
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 12345,
                  trackName: 'Test Track',
                  artistName: 'Test Artist',
                  collectionName: 'Test Album',
                  trackTimeMillis: 180000,
                  primaryGenreName: 'Pop',
                  releaseDate: '2023-05-01T00:00:00Z',
                  trackNumber: 3,
                  trackCount: 12,
                },
              ],
            }),
        } as unknown as Response),
      ) as unknown as typeof fetch as unknown as typeof fetch

      try {
        registerInfoCommand(ctx)
        const { repliedTexts } = await dispatchMessage('/info 12345')

        expect(repliedTexts.length).toBe(1)
        expect(repliedTexts[0]).toContain('Test Track')
        expect(repliedTexts[0]).toContain('Test Artist')
        expect(repliedTexts[0]).toContain('Cached in Database')
        expect(repliedTexts[0]).toContain('24-bit')
      } finally {
        globalThis.fetch = originalFetch
      }
    })
  })

  describe('Health / Ping Command', () => {
    it('pings health services and updates status message', async () => {
      const originalFetch = globalThis.fetch
      globalThis.fetch = mock(() =>
        Promise.resolve({
          ok: true,
          status: 200,
        } as unknown as Response),
      ) as unknown as typeof fetch

      try {
        registerHealthCommand(ctx)
        const { repliedTexts } = await dispatchMessage('/ping')

        expect(repliedTexts[0]).toContain('Testing system health')
        expect(fakeTg.editMessage).toHaveBeenCalled()
      } finally {
        globalThis.fetch = originalFetch
      }
    })
  })

  describe('Help Command', () => {
    it('blocks unauthorized users', async () => {
      mockAuth.isAuthorized = mock(() => Promise.resolve(false))
      registerHelpCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/help')
      expect(repliedTexts.length).toBe(0)
    })

    it('renders general usage guide for regular users', async () => {
      mockAuth.isAdmin = mock(() => false)
      registerHelpCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/help')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('ALAC Bot Usage Guide')
      expect(repliedTexts[0]).not.toContain('Admin Commands:')
    })

    it('includes admin commands section for admin users', async () => {
      mockAuth.isAdmin = mock(() => true)
      registerHelpCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/help')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Admin Commands:')
    })
  })

  describe('Stats Command', () => {
    it('blocks non-admin users', async () => {
      mockAuth.isAdmin = mock(() => false)
      registerStatsCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/stats')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Access Restricted')
    })

    it('renders formatted stats for admin users', async () => {
      mockAuth.isAdmin = mock(() => true)
      registerStatsCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/stats')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('ALAC Bot Analytics')
      expect(mockService.getStats).toHaveBeenCalled()
    })
  })

  describe('Index Command', () => {
    it('blocks non-admin users', async () => {
      mockAuth.isAdmin = mock(() => false)
      registerIndexCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/index')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Access Restricted')
    })

    it('runs dump channel index for admin users', async () => {
      mockAuth.isAdmin = mock(() => true)
      fakeTg.iterHistory = mock(
        async function* () {},
      ) as unknown as typeof fakeTg.iterHistory

      registerIndexCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/index')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Initializing Dump Channel Sync')
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })
  })

  describe('Search Command & Callbacks', () => {
    it('blocks unauthorized users', async () => {
      mockAuth.isAuthorized = mock(() => Promise.resolve(false))
      registerSearchCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/search test')
      expect(repliedTexts.length).toBe(0)
    })

    it('shows usage when search query is empty', async () => {
      registerSearchCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/search')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Search Music:')
    })

    it('notifies when no tracks match query', async () => {
      mockService.searchCachedTracks = mock(() => Promise.resolve([]))
      registerSearchCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/search NonExistentXYZ')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('No tracks found')
    })

    it('renders search results when tracks match query', async () => {
      mockService.searchCachedTracks = mock(() =>
        Promise.resolve([
          {
            id: 1,
            appleTrackId: '12345',
            messageId: 42,
            fileId: 'fid',
            fileUniqueId: 'uid',
            title: 'Test Song',
            artist: 'Test Artist',
            album: 'Test Album',
            duration: 200,
            bitDepth: 24,
            sampleRate: 48000,
            genre: 'Pop',
            releaseDate: '2021-01-01',
            trackNumber: 1,
            trackCount: 1,
            createdAt: new Date(),
            updatedAt: new Date(),
          },
        ]),
      )
      registerSearchCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/search Test')
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Search results for')
      expect(repliedTexts[0]).toContain('Test Song')
    })

    it('handles search_close callback query', async () => {
      registerSearchCommand(ctx)
      const { cb } = await dispatchCallback('search_close')
      expect(cb.answer).toHaveBeenCalled()
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
    })

    it('handles rip: callback query and enqueues lossless rip', async () => {
      registerSearchCommand(ctx)
      const { cb } = await dispatchCallback('rip:999888')

      expect(cb.answer).toHaveBeenCalledWith(
        expect.objectContaining({ text: expect.stringContaining('Queuing') }),
      )
      expect(mockQueue.enqueue).toHaveBeenCalled()
      expect(mockRipper.rip).toHaveBeenCalledWith(
        '999888',
        expect.any(Function),
      )
      expect(mockService.saveTrack).toHaveBeenCalled()
      expect(fakeTg.sendCopy).toHaveBeenCalledWith(
        expect.objectContaining({
          caption: { text: '' },
        }),
      )
      expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(100, [101, 42])
    })

    it('handles dl: callback query and delivers cached track', async () => {
      registerSearchCommand(ctx)
      const { answeredTexts } = await dispatchCallback('dl:12345')
      expect(answeredTexts).toContain(
        '⚡ Delivering lossless track from cache!',
      )
      expect(fakeTg.sendCopy).toHaveBeenCalledWith(
        expect.objectContaining({
          caption: { text: '' },
        }),
      )
    })
  })
})
