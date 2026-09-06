import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

import type { Message, TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import {
  fetchCandidateBySource,
  fetchChartsAlbum,
  fetchSearchAlbum,
  fetchWildAlbum,
  registerRandomCommand,
} from '@/modules/alac/commands/random.ts'
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

describe('Random Album Command & Explorer (/random)', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockService: IAlacService
  let mockQueue: IRipQueue
  let mockRipper: ITrackRipper
  let ctx: CommandContext

  let originalFetch: typeof globalThis.fetch

  beforeEach(() => {
    originalFetch = globalThis.fetch

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
      isAdmin: mock((userId: number) => userId === 1),
      isAuthorized: mock((userId: number) =>
        Promise.resolve(userId === 1 || userId === 2),
      ),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    mockService = {
      findCachedTrack: mock(() => Promise.resolve(null)),
      findTrackByFileUniqueId: mock(() => Promise.resolve(null)),
      saveCachedTrack: mock(() => Promise.resolve(null as never)),
      getStats: mock(() =>
        Promise.resolve({ totalTracks: 0, totalSize: 0, cacheHits: 0 }),
      ),
      searchTracks: mock(() => Promise.resolve([])),
      recordSearchQuery: mock(() => Promise.resolve()),
    } as unknown as IAlacService

    mockQueue = {
      enqueue: mock(async (task) => {
        const signal = new AbortController().signal
        await task(signal)
      }),
      cancel: mock(() => false),
      getActive: mock(() => null),
      getPending: mock(() => []),
      size: 0,
      activeCount: 0,
    } as unknown as IRipQueue

    mockRipper = {
      ripTrack: mock(() =>
        Promise.resolve({
          filePath: '/tmp/test_random_track.m4a',
          title: 'Test Song',
          artist: 'Test Artist',
          album: 'Test Album',
          duration: 180,
          bitDepth: 16,
          sampleRate: 44100,
          codec: 'alac',
          genre: 'Rock',
          releaseDate: '2024-01-01',
          trackNumber: 1,
        }),
      ),
    } as unknown as ITrackRipper

    ctx = {
      dp,
      tg: fakeTg,
      service: mockService,
      ripper: mockRipper,
      queue: mockQueue,
      auth: mockAuth,
    }
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  async function dispatchMessage(text: string, userId = 1) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const repliedOptions: unknown[] = []
    const msg = {
      _name: 'new_message',
      id: 10,
      text,
      sender: { id: userId, displayName: `User${userId}` },
      chat: { id: userId, displayName: `User${userId}`, isGroup: false },
      getReplyTo: mock(() => Promise.resolve(null)),
      replyText: mock((t: { text?: string } | string, opts?: unknown) => {
        const str = typeof t === 'string' ? t : t.text || ''
        repliedTexts.push(str)
        repliedOptions.push(opts)
        return Promise.resolve({
          id: 20,
          chat: { id: userId },
        })
      }),
    } as unknown as Message

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return { msg, repliedTexts, repliedOptions }
  }

  async function dispatchCallbackQuery(data: string, userId = 1) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('callback_query') ?? []

    const answered: Array<{ text?: string; alert?: boolean }> = []
    const query = {
      _name: 'callback_query',
      id: 'cb1',
      data,
      dataStr: data,
      raw: { data: Buffer.from(data) },
      user: { id: userId, displayName: `User${userId}` },
      chat: { id: userId, displayName: `User${userId}` },
      messageId: 50,
      answer: mock((opts?: { text?: string; alert?: boolean }) => {
        answered.push(opts || {})
        return Promise.resolve()
      }),
    }

    for (const h of handlers) {
      if (await h.check(query)) {
        await h.callback(query)
      }
    }

    return { answered }
  }

  describe('Discovery Sources Logic', () => {
    it('fetchChartsAlbum fetches and randomly picks from RSS feed', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              feed: {
                results: [
                  {
                    id: '1001',
                    name: 'Chart Album One',
                    artistName: 'Top Artist',
                    url: 'https://music.apple.com/us/album/chart-album-one/1001',
                    artworkUrl100: 'https://example.com/art.jpg',
                    releaseDate: '2024-05-01',
                    genres: [{ name: 'Pop' }],
                  },
                ],
              },
            }),
            { status: 200 },
          ),
        ),
      ) as unknown as typeof fetch

      const result = await fetchChartsAlbum('us')
      expect(result.id).toBe('1001')
      expect(result.title).toBe('Chart Album One')
      expect(result.artist).toBe('Top Artist')
      expect(result.genre).toBe('Pop')
    })

    it('fetchSearchAlbum queries iTunes catalog and picks an album', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              results: [
                {
                  collectionId: 2002,
                  collectionName: 'Wild Search Album',
                  artistName: 'Search Artist',
                  collectionViewUrl:
                    'https://music.apple.com/us/album/wild/2002',
                  artworkUrl100: 'https://example.com/wild.jpg',
                  releaseDate: '2023-01-01',
                  primaryGenreName: 'Rock',
                  trackCount: 10,
                },
              ],
            }),
            { status: 200 },
          ),
        ),
      ) as unknown as typeof fetch

      const result = await fetchSearchAlbum('rock', 'us')
      expect(result.id).toBe('2002')
      expect(result.title).toBe('Wild Search Album')
      expect(result.trackCount).toBe(10)
    })

    it('fetchCandidateBySource routes correctly to charts and wild', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              results: [
                {
                  collectionId: 3003,
                  collectionName: 'Source Test Album',
                  artistName: 'Artist 3',
                  collectionViewUrl:
                    'https://music.apple.com/us/album/source/3003',
                },
              ],
            }),
            { status: 200 },
          ),
        ),
      ) as unknown as typeof fetch

      const wildAlbum = await fetchWildAlbum('us')
      expect(wildAlbum.id).toBe('3003')
      const res = await fetchCandidateBySource('rock')
      expect(res.id).toBe('3003')
    })
  })

  describe('Command Handler (/random)', () => {
    it('ignores unauthorized users', async () => {
      registerRandomCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/random', 999)
      expect(repliedTexts.length).toBe(0)
    })

    it('blocks authorized non-admin users with access restricted notice', async () => {
      registerRandomCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/random', 2)
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Access Restricted')
    })

    it('shows discovery source selection menu when admin runs /random without arguments', async () => {
      registerRandomCommand(ctx)
      const { repliedTexts, repliedOptions } = await dispatchMessage(
        '/random',
        1,
      )

      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Apple Music Random Album Explorer')
      expect(
        (repliedOptions[0] as { replyMarkup?: unknown })?.replyMarkup,
      ).toBeDefined()
    })

    it('directly resolves preview card when admin runs /random charts', async () => {
      globalThis.fetch = mock((url: string | URL | Request) => {
        const u = String(url)
        if (u.includes('marketingtools.apple.com')) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                feed: {
                  results: [
                    {
                      id: '5555',
                      name: 'Top Hit Album',
                      artistName: 'Chart Topper',
                      url: 'https://music.apple.com/us/album/top/5555',
                      releaseDate: '2024-02-02',
                      genres: [{ name: 'Pop' }],
                    },
                  ],
                },
              }),
              { status: 200 },
            ),
          )
        }
        // iTunes lookup for album
        return Promise.resolve(
          new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'collection',
                  collectionType: 'Album',
                  collectionId: 5555,
                  collectionName: 'Top Hit Album',
                  artistName: 'Chart Topper',
                  primaryGenreName: 'Pop',
                  releaseDate: '2024-02-02',
                  trackCount: 2,
                },
                {
                  wrapperType: 'track',
                  trackId: 1,
                  trackName: 'Track 1',
                  artistName: 'Chart Topper',
                  collectionName: 'Top Hit Album',
                  trackTimeMillis: 180000,
                },
                {
                  wrapperType: 'track',
                  trackId: 2,
                  trackName: 'Track 2',
                  artistName: 'Chart Topper',
                  collectionName: 'Top Hit Album',
                  trackTimeMillis: 200000,
                },
              ],
            }),
            { status: 200 },
          ),
        )
      }) as unknown as typeof fetch

      registerRandomCommand(ctx)
      const { repliedTexts } = await dispatchMessage('/random charts', 1)

      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Discovering random album')
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })
  })

  describe('Callback Queries (random:*)', () => {
    it('blocks non-admin users from tapping callback buttons', async () => {
      registerRandomCommand(ctx)
      const { answered } = await dispatchCallbackQuery('random:src:charts', 2)

      expect(answered.length).toBe(1)
      expect(answered[0].text).toContain('Access restricted')
      expect(answered[0].alert).toBe(true)
    })

    it('handles random:close by deleting message', async () => {
      registerRandomCommand(ctx)
      await dispatchCallbackQuery('random:close', 1)

      expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(1, [50])
    })

    it('handles random:menu by returning to sources menu', async () => {
      registerRandomCommand(ctx)
      await dispatchCallbackQuery('random:menu', 1)

      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('handles random:src:charts and updates to preview card', async () => {
      globalThis.fetch = mock((url: string | URL | Request) => {
        const u = String(url)
        if (u.includes('marketingtools.apple.com')) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                feed: {
                  results: [
                    {
                      id: '8888',
                      name: 'Chart Callback Album',
                      artistName: 'Artist 88',
                      url: 'https://music.apple.com/us/album/8888',
                      releaseDate: '2024-06-01',
                      genres: [{ name: 'Electronic' }],
                    },
                  ],
                },
              }),
              { status: 200 },
            ),
          )
        }
        return Promise.resolve(
          new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'collection',
                  collectionType: 'Album',
                  collectionId: 8888,
                  collectionName: 'Chart Callback Album',
                  artistName: 'Artist 88',
                  primaryGenreName: 'Electronic',
                  releaseDate: '2024-06-01',
                  trackCount: 1,
                },
                {
                  wrapperType: 'track',
                  trackId: 10,
                  trackName: 'Track 10',
                  artistName: 'Artist 88',
                  collectionName: 'Chart Callback Album',
                  trackTimeMillis: 180000,
                },
              ],
            }),
            { status: 200 },
          ),
        )
      }) as unknown as typeof fetch

      registerRandomCommand(ctx)
      const { answered } = await dispatchCallbackQuery('random:src:charts', 1)

      expect(answered.length).toBe(1)
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('handles random:dump by deleting preview and queuing dump pipeline in cache-only mode', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'collection',
                  collectionType: 'Album',
                  collectionId: 7777,
                  collectionName: 'Dump Target Album',
                  artistName: 'Dump Target Artist',
                  primaryGenreName: 'Alternative',
                  releaseDate: '2024-03-01',
                  trackCount: 1,
                },
                {
                  wrapperType: 'track',
                  trackId: 701,
                  trackName: 'T701',
                  artistName: 'Dump Target Artist',
                  collectionName: 'Dump Target Album',
                  trackTimeMillis: 150000,
                },
              ],
            }),
            { status: 200 },
          ),
        ),
      ) as unknown as typeof fetch

      registerRandomCommand(ctx)
      const { answered } = await dispatchCallbackQuery('random:dump:7777:us', 1)

      expect(answered.length).toBe(1)
      expect(answered[0].text).toContain('Queuing album dump')
      expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(1, [50])
      expect(fakeTg.sendText).toHaveBeenCalled()
      expect(mockQueue.enqueue).toHaveBeenCalled()
    })
  })
})
