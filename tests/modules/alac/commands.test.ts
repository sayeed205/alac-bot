import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerCleanCommand } from '@/modules/alac/commands/clean.ts'
import { registerDeleteCommand } from '@/modules/alac/commands/delete.ts'
import { registerHealthCommand } from '@/modules/alac/commands/health.ts'
import { registerInfoCommand } from '@/modules/alac/commands/info.ts'
import { registerQueueCommand } from '@/modules/alac/commands/queue.ts'
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
  let dp: Dispatcher<TelegramClient>
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
            createdAt: new Date(),
            updatedAt: new Date(),
          })
        }
        return Promise.resolve(null)
      }),
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
          cacheHitRatio: '75.0',
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
      enqueue: mock(() => Promise.resolve()),
      getPendingCount: mock(() => 0),
      isProcessing: mock(() => false),
    }

    mockRipper = {
      rip: mock(() => Promise.resolve({} as never)),
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
      )

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
      registerHealthCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/ping')

      expect(repliedTexts[0]).toContain('Testing system health')
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })
  })
})
