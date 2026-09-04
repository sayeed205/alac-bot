import { beforeEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'

import type { Message, TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { activeJobs, registerRipCommand } from '@/modules/alac/commands/rip.ts'
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
    activeJobs.clear()

    fakeTg = {
      sendCopy: mock(() => Promise.resolve({ id: 999 })),
      sendText: mock(() => Promise.resolve({ id: 123 })),
      getMe: mock(() => Promise.resolve({ username: 'alac_test_bot' })),
      sendMedia: mock(() =>
        Promise.resolve({
          id: 500,
          media: {
            type: 'audio',
            fileId: 'uploaded_file_id',
            uniqueFileId: 'uploaded_unique_id',
          },
        }),
      ),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      downloadToFile: mock(() => Promise.resolve()),
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
      enqueue: mock(async (task, options) =>
        task(options?.signal || new AbortController().signal),
      ) as unknown as IRipQueue['enqueue'],
      getPendingCount: mock(() => 0),
      isProcessing: mock(() => false),
      clear: mock(() => {}),
    }

    mockRipper = {
      rip: mock(
        async (
          _id: string,
          onProgress?: (msg: string) => void,
          _storefront?: string,
          signal?: AbortSignal,
        ) => {
          if (signal?.aborted) {
            throw new Error('Download was cancelled')
          }
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
        },
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

  async function dispatchMessage(
    text: string,
    userId = 1,
    chatType = 'user',
    extraProps: Record<string, unknown> = {},
  ) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const chatId = chatType === 'user' ? userId : 9999
    const msg = {
      _name: 'new_message',
      id: 10,
      text,
      sender: { id: userId, displayName: `User${userId}` },
      chat: {
        id: chatId,
        displayName: 'Test Peer',
        isGroup: chatType !== 'user',
      },
      getReplyTo: mock(() => Promise.resolve(null)),
      replyText: mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({
          id: 20,
          chat: { id: chatId },
        })
      }),
      ...extraProps,
    } as unknown as Message

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return { msg, repliedTexts }
  }

  async function dispatchCallbackQuery(
    data: string,
    userId = 1,
    displayName = 'Requester',
  ) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('callback_query') ?? []

    const answered: Array<{ text?: string; alert?: boolean }> = []
    const query = {
      _name: 'callback_query',
      id: 'cb1',
      dataStr: data,
      raw: { data: new TextEncoder().encode(data) },
      user: { id: userId, displayName },
      chat: { id: 1 },
      messageId: 20,
      answer: mock((params: { text?: string; alert?: boolean } = {}) => {
        answered.push(params)
        return Promise.resolve()
      }),
    }

    for (const h of handlers) {
      if (await h.check(query)) {
        await h.callback(query)
      }
    }

    return { query, answered }
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
      expect.anything(),
    )
    expect(fakeTg.sendMedia).toHaveBeenCalled()
    expect(mockService.saveTrack).toHaveBeenCalled()
    expect(fakeTg.sendCopy).toHaveBeenCalled()
    expect(fakeTg.sendCopy).toHaveBeenCalledWith(
      expect.objectContaining({
        toChatId: 1,
      }),
    )
    expect(mockService.logRequest).toHaveBeenCalled()
  })

  it('routes audio delivery to user DM when requested in a group chat', async () => {
    registerRipCommand(ctx)
    await dispatchMessage('/alac 12345', 42, 'supergroup')

    // Initial DM notification sent to the user
    expect(fakeTg.sendText).toHaveBeenCalledWith(
      42,
      expect.anything(),
      expect.objectContaining({ silent: true }),
    )
    // Files copied to user DM (chat ID 42) instead of the group
    expect(fakeTg.sendCopy).toHaveBeenCalledWith(
      expect.objectContaining({
        toChatId: 42,
      }),
    )
  })

  it('prompts to start bot in DM if user in group has not started private chat', async () => {
    fakeTg.sendText = mock(() =>
      Promise.reject(new Error('BOT_CANNOT_INITIATE_DM')),
    )

    registerRipCommand(ctx)
    const { repliedTexts } = await dispatchMessage(
      '/alac 12345',
      42,
      'supergroup',
    )

    expect(repliedTexts.length).toBe(1)
    expect(repliedTexts[0]).toContain('Direct Message Required')
    expect(mockRipper.rip).not.toHaveBeenCalled()
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

  it('stops remaining batch immediately when mirror server is offline (circuit breaker)', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('itunes.apple.com')) {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 888888,
                collectionName: 'Offline Album',
                artistName: 'Artist',
              },
              { wrapperType: 'track', trackId: 201, trackName: 'T1' },
              { wrapperType: 'track', trackId: 202, trackName: 'T2' },
              { wrapperType: 'track', trackId: 203, trackName: 'T3' },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response('ok')
    }) as unknown as typeof fetch

    mockRipper.rip = mock(() =>
      Promise.reject(new Error('Mirror /status check timed out after 8000ms')),
    )

    try {
      registerRipCommand(ctx)
      await dispatchMessage(
        '/alac https://music.apple.com/us/album/offline/888888',
      )

      // Instead of attempting all 3 tracks, circuit breaker stops after 1 attempt
      expect(mockRipper.rip).toHaveBeenCalledTimes(1)
      expect(fakeTg.editMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          text: expect.objectContaining({
            text: expect.stringContaining('Mirror service offline'),
          }),
        }),
      )
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

  it('works with command aliases /batch, /rip, /dl, and /download', async () => {
    registerRipCommand(ctx)

    await dispatchMessage('/batch 12345')
    expect(mockRipper.rip).toHaveBeenCalledWith(
      '12345',
      expect.anything(),
      undefined,
      expect.anything(),
    )

    await dispatchMessage('/rip 12345')
    expect(mockRipper.rip).toHaveBeenCalledWith(
      '12345',
      expect.anything(),
      undefined,
      expect.anything(),
    )

    await dispatchMessage('/dl 12345')
    expect(mockRipper.rip).toHaveBeenCalledWith(
      '12345',
      expect.anything(),
      undefined,
      expect.anything(),
    )

    await dispatchMessage('/download 12345')
    expect(mockRipper.rip).toHaveBeenCalledWith(
      '12345',
      expect.anything(),
      undefined,
      expect.anything(),
    )
  })

  describe('Download Cancellation', () => {
    it('allows requester to cancel ongoing download via cancel: callback query', async () => {
      let resolveRip: () => void
      const ripWaitPromise = new Promise<void>((r) => {
        resolveRip = r
      })

      mockRipper.rip = mock(async (_id, _onProgress, _sf, signal) => {
        return new Promise((resolve, reject) => {
          signal?.addEventListener('abort', () => {
            reject(new Error('Download was cancelled'))
          })
          ripWaitPromise.then(() => {
            resolve({
              filePath: '/tmp/test.m4a',
              title: 'Song',
              artist: 'Artist',
              album: 'Album',
              duration: 200,
              bitDepth: 24,
              sampleRate: 96000,
              codec: 'alac',
              genre: 'Pop',
              releaseDate: '2023',
              trackNumber: 1,
              trackCount: 1,
            })
          })
        })
      })

      registerRipCommand(ctx)
      // Start download as user 55
      const ripPromise = dispatchMessage('/alac 99999', 55)

      // Allow microtasks to run so job is registered
      await new Promise((r) => setTimeout(r, 10))

      const activeJob = Array.from(activeJobs.values())[0]
      expect(activeJob).toBeDefined()
      expect(activeJob?.userId).toBe(55)

      // User 55 cancels the job via callback query
      const { answered } = await dispatchCallbackQuery(
        `cancel:${activeJob?.id}`,
        55,
        'Sayeed',
      )

      expect(answered[0]?.text).toContain('Download cancelled')
      expect(activeJob?.isCancelled).toBe(true)

      resolveRip?.()
      await ripPromise

      // Edit message should show Download Cancelled
      expect(fakeTg.editMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          text: expect.objectContaining({
            text: expect.stringContaining('Download Cancelled'),
          }),
        }),
      )
    })

    it('denies cancellation attempt from non-requester non-admin user', async () => {
      let resolveRip: () => void
      const ripWaitPromise = new Promise<void>((r) => {
        resolveRip = r
      })

      mockRipper.rip = mock(async (_id, _onProgress, _sf, signal) => {
        return new Promise((resolve, reject) => {
          signal?.addEventListener('abort', () => {
            reject(new Error('Download was cancelled'))
          })
          ripWaitPromise.then(() => {
            resolve({
              filePath: '/tmp/test.m4a',
              title: 'Song',
              artist: 'Artist',
              album: 'Album',
              duration: 200,
              bitDepth: 24,
              sampleRate: 96000,
              codec: 'alac',
              genre: 'Pop',
              releaseDate: '2023',
              trackNumber: 1,
              trackCount: 1,
            })
          })
        })
      })

      registerRipCommand(ctx)
      // Started by user 55
      const ripPromise = dispatchMessage('/alac 99999', 55)
      await new Promise((r) => setTimeout(r, 10))

      const activeJob = Array.from(activeJobs.values())[0]
      expect(activeJob).toBeDefined()

      // User 77 (not admin, not requester) attempts to cancel
      const { answered } = await dispatchCallbackQuery(
        `cancel:${activeJob?.id}`,
        77,
        'Stranger',
      )

      expect(answered[0]?.alert).toBe(true)
      expect(answered[0]?.text).toContain('Only the person who requested')
      expect(activeJob?.isCancelled).toBe(false)

      resolveRip?.()
      await ripPromise
    })

    it('allows admin to cancel another user download', async () => {
      let resolveRip: () => void
      const ripWaitPromise = new Promise<void>((r) => {
        resolveRip = r
      })

      mockRipper.rip = mock(async (_id, _onProgress, _sf, signal) => {
        return new Promise((resolve, reject) => {
          signal?.addEventListener('abort', () => {
            reject(new Error('Download was cancelled'))
          })
          ripWaitPromise.then(() => {
            resolve({
              filePath: '/tmp/test.m4a',
              title: 'Song',
              artist: 'Artist',
              album: 'Album',
              duration: 200,
              bitDepth: 24,
              sampleRate: 96000,
              codec: 'alac',
              genre: 'Pop',
              releaseDate: '2023',
              trackNumber: 1,
              trackCount: 1,
            })
          })
        })
      })

      registerRipCommand(ctx)
      // Started by user 55
      const ripPromise = dispatchMessage('/alac 99999', 55)
      await new Promise((r) => setTimeout(r, 10))

      const activeJob = Array.from(activeJobs.values())[0]
      expect(activeJob).toBeDefined()

      // Admin (user 1) cancels
      const { answered } = await dispatchCallbackQuery(
        `cancel:${activeJob?.id}`,
        1,
        'AdminUser',
      )

      expect(answered[0]?.text).toContain('Download cancelled')
      expect(activeJob?.isCancelled).toBe(true)

      resolveRip?.()
      await ripPromise
    })

    it('cancels active download via /cancel command', async () => {
      let resolveRip: () => void
      const ripWaitPromise = new Promise<void>((r) => {
        resolveRip = r
      })

      mockRipper.rip = mock(async (_id, _onProgress, _sf, signal) => {
        return new Promise((resolve, reject) => {
          signal?.addEventListener('abort', () => {
            reject(new Error('Download was cancelled'))
          })
          ripWaitPromise.then(() => {
            resolve({
              filePath: '/tmp/test.m4a',
              title: 'Song',
              artist: 'Artist',
              album: 'Album',
              duration: 200,
              bitDepth: 24,
              sampleRate: 96000,
              codec: 'alac',
              genre: 'Pop',
              releaseDate: '2023',
              trackNumber: 1,
              trackCount: 1,
            })
          })
        })
      })

      registerRipCommand(ctx)
      // Started by user 55
      const ripPromise = dispatchMessage('/alac 99999', 55)
      await new Promise((r) => setTimeout(r, 10))

      const activeJob = Array.from(activeJobs.values())[0]
      expect(activeJob).toBeDefined()

      // User 55 types /cancel
      const { repliedTexts } = await dispatchMessage('/cancel', 55)
      expect(repliedTexts[0]).toContain('Download has been cancelled')
      expect(activeJob?.isCancelled).toBe(true)

      resolveRip?.()
      await ripPromise
    })
  })
})
