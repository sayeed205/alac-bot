import { beforeEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'

import type { Message, TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { registerRipCommand } from '@/modules/alac/commands/rip.ts'
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

describe('ALAC Rip Command Handler', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockService: IAlacService
  let mockQueue: IRipQueue
  let mockRipper: ITrackRipper
  let ctx: CommandContext

  beforeEach(() => {
    fakeTg = {
      sendCopy: mock(() => Promise.resolve({ id: 999 })),
      sendMedia: mock(() =>
        Promise.resolve({
          id: 500,
          media: {
            fileId: 'uploaded_file_id',
            uniqueId: 'uploaded_unique_id',
          },
        }),
      ),
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

    mockService = {
      findCachedTrack: mock(() => Promise.resolve(null)),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve({} as never)),
      searchCachedTracks: mock(() => Promise.resolve([])),
      logRequest: mock(() => Promise.resolve()),
      getStats: mock(() => Promise.resolve({} as never)),
      deleteTrack: mock(() => Promise.resolve(true)),
      getAllTrackIds: mock(() => Promise.resolve([])),
      deleteTracksNotIn: mock(() => Promise.resolve(0)),
    }

    mockQueue = {
      enqueue: mock(async (task) =>
        task(new AbortController().signal),
      ) as unknown as IRipQueue['enqueue'],
      getPendingCount: mock(() => 0),
      isProcessing: mock(() => false),
      clear: mock(() => {}),
    }

    mockRipper = {
      rip: mock(async (_id: string, onProgress?: (msg: string) => void) => {
        onProgress?.('Decrypting...')
        const dummyFile = '/tmp/test_track_rip.m4a'
        fs.writeFileSync(dummyFile, 'dummy audio data')
        return {
          filePath: dummyFile,
          title: 'Ripped Song',
          artist: 'Ripped Artist',
          album: 'Ripped Album',
          duration: 215,
          bitDepth: 24,
          sampleRate: 96000,
          codec: 'alac',
          genre: 'Pop',
          releaseDate: '2023-01-01',
          trackNumber: 1,
          trackCount: 1,
        }
      }),
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

  async function dispatchMessage(text: string, userId = 1) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const msg = {
      _name: 'new_message',
      id: 10,
      text,
      sender: { id: userId },
      chat: { id: 100 },
      getReplyTo: mock(() => Promise.resolve(null)),
      replyText: mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 20, chat: { id: 100 } })
      }),
    } as unknown as Message

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return { msg, repliedTexts }
  }

  it('ignores unauthorized users', async () => {
    mockAuth.isAuthorized = mock(() => Promise.resolve(false))
    registerRipCommand(ctx)
    const { repliedTexts } = await dispatchMessage('/alac 12345', 999)
    expect(repliedTexts.length).toBe(0)
  })

  it('shows usage guide when input is invalid', async () => {
    registerRipCommand(ctx)
    const { repliedTexts } = await dispatchMessage('/alac')
    expect(repliedTexts.length).toBe(1)
    expect(repliedTexts[0]).toContain('Apple Music Lossless Ripper')
  })

  it('blocks non-admin users from using force re-rip', async () => {
    registerRipCommand(ctx)
    const { repliedTexts } = await dispatchMessage('/alac 12345 -f', 999)
    expect(repliedTexts.length).toBe(1)
    expect(repliedTexts[0]).toContain(
      'Force re-rip is restricted to the bot owner',
    )
  })

  it('delivers track directly on cache hit', async () => {
    mockService.findCachedTracks = mock(() =>
      Promise.resolve(
        new Map([
          [
            '12345',
            {
              id: 1,
              appleTrackId: '12345',
              messageId: 77,
              fileId: 'fid',
              fileUniqueId: 'uid',
              title: 'Cached Track',
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
            },
          ],
        ]),
      ),
    )

    registerRipCommand(ctx)
    await dispatchMessage('/alac 12345')

    expect(fakeTg.sendCopy).toHaveBeenCalled()
    expect(mockService.logRequest).toHaveBeenCalled()
    expect(mockRipper.rip).not.toHaveBeenCalled()
  })

  it('processes cache miss: queues job, rips track, uploads to dump channel, and sends copy', async () => {
    registerRipCommand(ctx)
    await dispatchMessage('/alac 12345')

    expect(mockQueue.enqueue).toHaveBeenCalled()
    expect(mockRipper.rip).toHaveBeenCalledWith(
      '12345',
      expect.any(Function),
      undefined,
    )
    expect(fakeTg.sendMedia).toHaveBeenCalled()
    expect(mockService.saveTrack).toHaveBeenCalled()
    expect(fakeTg.sendCopy).toHaveBeenCalled()
    expect(mockService.logRequest).toHaveBeenCalled()
  })

  it('handles album links and rips multiple tracks', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('itunes.apple.com')) {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 123456,
                collectionName: 'Album Title',
                artistName: 'Album Artist',
              },
              {
                wrapperType: 'track',
                trackId: 1,
                trackName: 'Song 1',
                artistName: 'Album Artist',
                collectionName: 'Album Title',
                trackTimeMillis: 180000,
              },
              {
                wrapperType: 'track',
                trackId: 2,
                trackName: 'Song 2',
                artistName: 'Album Artist',
                collectionName: 'Album Title',
                trackTimeMillis: 200000,
              },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response('ok')
    }) as unknown as typeof fetch

    try {
      registerRipCommand(ctx)
      await dispatchMessage(
        '/alac https://music.apple.com/us/album/test/123456',
      )

      expect(mockRipper.rip).toHaveBeenCalledTimes(2)
      expect(mockService.saveTrack).toHaveBeenCalledTimes(2)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  it('continues ripping remaining album tracks when one track fails', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('itunes.apple.com')) {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 999999,
                collectionName: 'Resilient Album',
                artistName: 'Album Artist',
              },
              {
                wrapperType: 'track',
                trackId: 101,
                trackName: 'Track 1',
                artistName: 'Album Artist',
                collectionName: 'Resilient Album',
                trackTimeMillis: 180000,
              },
              {
                wrapperType: 'track',
                trackId: 102,
                trackName: 'Track 2 (Broken)',
                artistName: 'Album Artist',
                collectionName: 'Resilient Album',
                trackTimeMillis: 200000,
              },
              {
                wrapperType: 'track',
                trackId: 103,
                trackName: 'Track 3',
                artistName: 'Album Artist',
                collectionName: 'Resilient Album',
                trackTimeMillis: 210000,
              },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response('ok')
    }) as unknown as typeof fetch

    mockRipper.rip = mock(async (id: string) => {
      if (id === '102') {
        throw new Error('DRM decryption error')
      }
      return {
        filePath: '/tmp/test_track_rip.m4a',
        title: `Track ${id}`,
        artist: 'Album Artist',
        album: 'Resilient Album',
        duration: 200,
        bitDepth: 24,
        sampleRate: 96000,
        codec: 'alac',
        genre: 'Pop',
        releaseDate: '2023-01-01',
        trackNumber: Number(id),
        trackCount: 3,
      }
    })

    try {
      registerRipCommand(ctx)
      await dispatchMessage(
        '/alac https://music.apple.com/us/album/resilient/999999',
      )

      expect(mockRipper.rip).toHaveBeenCalledTimes(3)
      expect(mockService.saveTrack).toHaveBeenCalledTimes(2)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  it('handles rip failure gracefully and reports error to user', async () => {
    mockRipper.rip = mock(() => Promise.reject(new Error('Decryption failed')))

    registerRipCommand(ctx)
    await dispatchMessage('/alac 12345')

    expect(fakeTg.editMessage).toHaveBeenCalled()
    expect(mockService.logRequest).toHaveBeenCalledWith(
      expect.objectContaining({
        status: 'failed',
        errorReason: 'Decryption failed',
      }),
    )
  })
})
