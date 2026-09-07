import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { Message, TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import {
  activeReports,
  activeReportsByTrack,
  checkUserRateLimit,
  extractTrackIdFromText,
  registerReportCommand,
  resetReportState,
} from '@/modules/alac/commands/report.ts'
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

describe('Track Issue Reporting (/report & /issue)', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockService: IAlacService
  let mockQueue: IRipQueue
  let mockRipper: ITrackRipper
  let ctx: CommandContext
  let sentTexts: string[]
  let editedTexts: string[]

  const sampleTrack = {
    id: 1,
    appleTrackId: '1717520442',
    messageId: 2799,
    fileId: 'file_abc',
    fileUniqueId: 'unique_xyz',
    title: 'Bishakto Manush',
    artist: 'Fossils',
    album: 'Fossils, Vol. 1',
    duration: 267,
    bitDepth: 16,
    sampleRate: 44100,
    genre: 'Rock',
    releaseDate: '2002-01-01',
    trackNumber: 1,
    trackCount: 8,
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(() => {
    resetReportState()
    sentTexts = []
    editedTexts = []

    fakeTg = {
      sendText: mock((_chatId: unknown, textObj: { text: string } | string) => {
        const text = typeof textObj === 'string' ? textObj : textObj.text
        sentTexts.push(text)
        return Promise.resolve({ id: 101 })
      }),
      editMessage: mock((params: { text: { text: string } | string }) => {
        const text =
          typeof params.text === 'string' ? params.text : params.text.text
        editedTexts.push(text)
        return Promise.resolve({ id: 1 })
      }),
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
      findCachedTrack: mock((id: string) => {
        if (id === '1717520442') {
          return Promise.resolve(sampleTrack)
        }
        return Promise.resolve(null)
      }),
      findTrackByFileUniqueId: mock((uniqueId: string) => {
        if (uniqueId === 'unique_xyz') {
          return Promise.resolve(sampleTrack)
        }
        return Promise.resolve(null)
      }),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve(sampleTrack as never)),
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
      rip: mock(async () => ({
        filePath: '/tmp/test.m4a',
        title: sampleTrack.title,
        artist: sampleTrack.artist,
        album: sampleTrack.album,
        duration: sampleTrack.duration,
        codec: 'alac',
        bitDepth: 16,
        sampleRate: 44100,
        genre: 'Rock',
        releaseDate: '2002-01-01',
        trackNumber: 1,
        trackCount: 8,
      })),
    }

    ctx = {
      dp,
      tg: fakeTg,
      service: mockService,
      ripper: mockRipper,
      queue: mockQueue,
      auth: mockAuth,
    }

    registerReportCommand(ctx)
  })

  describe('extractTrackIdFromText & checkUserRateLimit', () => {
    it('extracts track id from Apple Music song URLs and bare IDs', () => {
      expect(
        extractTrackIdFromText(
          'https://music.apple.com/us/album/bishakto-manush/1717520442?i=1717520442',
        ),
      ).toBe('1717520442')
      expect(
        extractTrackIdFromText(
          'https://music.apple.com/in/song/wild-sun/1771716134',
        ),
      ).toBe('1771716134')
      expect(extractTrackIdFromText('1717520442')).toBe('1717520442')
      expect(extractTrackIdFromText('not a link')).toBeNull()
    })

    it('enforces 5 reports per hour rate limit per user', () => {
      const userId = 500
      for (let i = 0; i < 5; i++) {
        expect(checkUserRateLimit(userId)).toBe(true)
      }
      expect(checkUserRateLimit(userId)).toBe(false)
    })
  })

  describe('Command Invocations (/report & /issue)', () => {
    const dispatchMessage = async (msgObj: Record<string, unknown>) => {
      const internalDp = dp as unknown as DispatcherInternal
      const group0 = internalDp._groups.get(0)
      const handlers = group0?.get('new_message') ?? []
      for (const h of handlers) {
        if (await h.check(msgObj)) {
          await h.callback(msgObj)
        }
      }
    }

    it('ignores unauthorized users', async () => {
      mockAuth.isAuthorized = mock(() => Promise.resolve(false))
      mockAuth.isAdmin = mock(() => false)

      const replyTextMock = mock(() => Promise.resolve({ id: 1 }))
      const msg = {
        _name: 'new_message',
        text: '/report',
        command: ['report'],
        sender: { id: 999 },
        chat: { id: 100 },
        replyText: replyTextMock,
        getReplyTo: mock(() => Promise.resolve(null)),
      }

      await dispatchMessage(msg)
      expect(replyTextMock).not.toHaveBeenCalled()
    })

    it('shows usage guide when called without reply and without valid arguments', async () => {
      const repliedTexts: string[] = []
      const replyTextMock = mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1 })
      })
      const msg = {
        _name: 'new_message',
        text: '/report',
        command: ['report'],
        sender: { id: 10, displayName: 'John' },
        chat: { id: 100 },
        replyText: replyTextMock,
        getReplyTo: mock(() => Promise.resolve(null)),
      }

      await dispatchMessage(msg)
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Report a Track Issue')
      expect(repliedTexts[0]).toContain('Reply to any song')
    })

    it('prompts with reason selection keyboard when replying to audio without text', async () => {
      const repliedTexts: string[] = []
      const replyOptionsList: Array<{ replyMarkup?: unknown } | undefined> = []
      const replyTextMock = mock(
        (t: { text: string } | string, options?: { replyMarkup?: unknown }) => {
          const str = typeof t === 'string' ? t : t.text
          repliedTexts.push(str)
          replyOptionsList.push(options)
          return Promise.resolve({ id: 1 })
        },
      )
      const msg = {
        _name: 'new_message',
        text: '/report',
        command: ['report'],
        sender: { id: 10, displayName: 'Alice' },
        chat: { id: 100 },
        replyText: replyTextMock,
        getReplyTo: mock(() =>
          Promise.resolve({
            media: {
              type: 'audio',
              uniqueFileId: 'unique_xyz',
            },
          } as unknown as Message),
        ),
      }

      await dispatchMessage(msg)
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Report Track Issue')
      expect(repliedTexts[0]).toContain('Bishakto Manush')
      expect(replyOptionsList[0]?.replyMarkup).toBeDefined()
    })

    it('immediately submits report to admin when user provides description', async () => {
      const repliedTexts: string[] = []
      const replyTextMock = mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1 })
      })
      const msg = {
        _name: 'new_message',
        text: '/report sound cuts off at 2:15',
        command: ['report'],
        sender: { id: 10, displayName: 'Alice' },
        chat: { id: 100 },
        replyText: replyTextMock,
        getReplyTo: mock(() =>
          Promise.resolve({
            media: {
              type: 'audio',
              uniqueFileId: 'unique_xyz',
            },
          } as unknown as Message),
        ),
      }

      await dispatchMessage(msg)

      // Sends notification card to admin
      expect(sentTexts.length).toBe(1)
      expect(sentTexts[0]).toContain('New Track Issue Report')
      expect(sentTexts[0]).toContain('Alice')
      expect(sentTexts[0]).toContain('Bishakto Manush')
      expect(sentTexts[0]).toContain('sound cuts off at 2:15')

      // Confirms to user
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Report Submitted')

      // Track registered as pending report
      expect(activeReportsByTrack.has('1717520442')).toBe(true)
    })

    it('prevents duplicate reports for the same track', async () => {
      activeReportsByTrack.set('1717520442', 'existing_report')

      const repliedTexts: string[] = []
      const replyTextMock = mock((t: { text: string } | string) => {
        const str = typeof t === 'string' ? t : t.text
        repliedTexts.push(str)
        return Promise.resolve({ id: 1 })
      })
      const msg = {
        _name: 'new_message',
        text: '/report 1717520442 corrupted',
        command: ['report'],
        sender: { id: 10, displayName: 'Bob' },
        chat: { id: 100 },
        replyText: replyTextMock,
        getReplyTo: mock(() => Promise.resolve(null)),
      }

      await dispatchMessage(msg)
      expect(repliedTexts.length).toBe(1)
      expect(repliedTexts[0]).toContain('Already Under Review')
      expect(sentTexts.length).toBe(0)
    })
  })

  describe('Callback Queries (report:*)', () => {
    const dispatchCallback = async (cbObj: Record<string, unknown>) => {
      const internalDp = dp as unknown as DispatcherInternal
      const group0 = internalDp._groups.get(0)
      const handlers = group0?.get('callback_query') ?? []
      for (const h of handlers) {
        if (await h.check(cbObj)) {
          await h.callback(cbObj)
        }
      }
    }

    it('handles report:cancel by deleting prompt message', async () => {
      const answerMock = mock(() => Promise.resolve())
      const query = {
        _name: 'callback_query',
        dataStr: 'report:cancel',
        raw: { data: Buffer.from('report:cancel') },
        user: { id: 10 },
        chat: { id: 100 },
        messageId: 55,
        answer: answerMock,
      }

      await dispatchCallback(query)
      expect(answerMock).toHaveBeenCalled()
      expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(100, [55])
    })

    it('handles user submission via preset button report:sub:1717520442:corrupted', async () => {
      const answerMock = mock(() => Promise.resolve())
      const query = {
        _name: 'callback_query',
        dataStr: 'report:sub:1717520442:corrupted',
        raw: { data: Buffer.from('report:sub:1717520442:corrupted') },
        user: { id: 10, displayName: 'Charlie' },
        chat: { id: 100 },
        messageId: 55,
        answer: answerMock,
      }

      await dispatchCallback(query)
      expect(sentTexts.length).toBe(1)
      expect(sentTexts[0]).toContain("Corrupted / Won't play")
      expect(editedTexts.length).toBe(1)
      expect(editedTexts[0]).toContain('Report Submitted')
      expect(activeReportsByTrack.has('1717520442')).toBe(true)
    })

    it('blocks non-admin from executing admin action callbacks', async () => {
      const answerMock = mock(() => Promise.resolve())
      const query = {
        _name: 'callback_query',
        dataStr: 'report:act:del:1717520442:rep1',
        raw: { data: Buffer.from('report:act:del:1717520442:rep1') },
        user: { id: 999 }, // non-admin
        chat: { id: 100 },
        messageId: 55,
        answer: answerMock,
      }

      await dispatchCallback(query)
      expect(answerMock).toHaveBeenCalledWith(
        expect.objectContaining({
          text: expect.stringContaining('Access restricted'),
        }),
      )
      expect(mockService.deleteTrack).not.toHaveBeenCalled()
    })

    it('allows admin to dismiss report via report:act:dismiss:rep1', async () => {
      activeReports.set('rep1', {
        id: 'rep1',
        trackId: '1717520442',
        reporterUserId: 10,
        reporterChatId: 100,
        reporterName: 'User',
        reason: 'test',
        trackTitle: 'Song',
        trackArtist: 'Artist',
        trackAlbum: 'Album',
        timestamp: Date.now(),
      })
      activeReportsByTrack.set('1717520442', 'rep1')

      const answerMock = mock(() => Promise.resolve())
      const query = {
        _name: 'callback_query',
        dataStr: 'report:act:dismiss:rep1',
        raw: { data: Buffer.from('report:act:dismiss:rep1') },
        user: { id: 1 }, // admin
        chat: { id: 1 },
        messageId: 55,
        answer: answerMock,
      }

      await dispatchCallback(query)
      expect(activeReports.has('rep1')).toBe(false)
      expect(activeReportsByTrack.has('1717520442')).toBe(false)
      expect(editedTexts.length).toBe(1)
      expect(editedTexts[0]).toContain('Report Dismissed')
    })

    it('allows admin to delete reported track via report:act:del:1717520442:rep1', async () => {
      activeReports.set('rep1', {
        id: 'rep1',
        trackId: '1717520442',
        reporterUserId: 10,
        reporterChatId: 100,
        reporterName: 'User',
        reason: 'corrupt',
        trackTitle: 'Bishakto Manush',
        trackArtist: 'Fossils',
        trackAlbum: 'Album',
        timestamp: Date.now(),
      })
      activeReportsByTrack.set('1717520442', 'rep1')

      const answerMock = mock(() => Promise.resolve())
      const query = {
        _name: 'callback_query',
        dataStr: 'report:act:del:1717520442:rep1',
        raw: { data: Buffer.from('report:act:del:1717520442:rep1') },
        user: { id: 1 }, // admin
        chat: { id: 1 },
        messageId: 55,
        answer: answerMock,
      }

      await dispatchCallback(query)
      expect(mockService.deleteTrack).toHaveBeenCalledWith('1717520442')
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
      expect(activeReports.has('rep1')).toBe(false)
      expect(editedTexts.length).toBe(1)
      expect(editedTexts[0]).toContain('Track Deleted')
    })
  })
})
