import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import type { IDbDumpService } from '@/db/dump.ts'
import {
  registerBackupCommands,
  registerExportCommand,
  registerImportCommand,
} from '@/modules/alac/commands/backup.ts'
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

function extractSentText(arg: unknown): string {
  if (typeof arg === 'string') return arg
  if (arg && typeof arg === 'object' && 'text' in arg) {
    return String((arg as { text: unknown }).text)
  }
  return String(arg)
}

describe('Backup Commands (/export & /import)', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockDumpService: IDbDumpService
  let ctx: CommandContext

  beforeEach(() => {
    fakeTg = {
      sendText: mock(() => Promise.resolve({ id: 101, chat: { id: 100 } })),
      sendMedia: mock(() => Promise.resolve({ id: 102 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      downloadToFile: mock((dest: string) => {
        const dummyGzip = Bun.gzipSync(Buffer.from('-- SQL DUMP'))
        return Bun.write(dest, dummyGzip).then(() => {})
      }),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    mockAuth = {
      isAdmin: mock((id: number) => id === 1),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    mockDumpService = {
      exportDump: mock(() =>
        Promise.resolve({
          buffer: new Uint8Array([1, 2, 3]),
          filename: 'alac_dump_test.sql.gz',
          stats: {
            usersCount: 10,
            tracksCount: 25,
            requestsCount: 100,
            bytes: 5120,
          },
        }),
      ),
      importDump: mock(() =>
        Promise.resolve({
          usersMerged: 5,
          tracksMerged: 20,
          requestsMerged: 50,
          durationMs: 45,
        }),
      ),
    }

    dp = Dispatcher.for(fakeTg)
    ctx = {
      dp,
      tg: fakeTg,
      service: {} as unknown as IAlacService,
      ripper: {} as unknown as ITrackRipper,
      queue: {} as unknown as IRipQueue,
      auth: mockAuth,
      dumpService: mockDumpService,
    }
  })

  async function dispatchMessage(opts: {
    text: string
    senderId?: number
    chatType?: 'user' | 'supergroup'
    replyToMessage?: {
      media?: {
        type: string
        fileName?: string
        name?: string
      }
    }
  }) {
    const fakeMsg = {
      text: opts.text,
      sender: { id: opts.senderId ?? 1, type: 'user' },
      chat: { id: 100, type: opts.chatType ?? 'user' },
      id: 42,
      replyText: mock(() => Promise.resolve()),
      getReplyTo: mock(() => Promise.resolve(opts.replyToMessage ?? null)),
    }

    const internal = dp as unknown as DispatcherInternal
    const handlers = internal._groups.get(0)?.get('new_message') || []
    for (const h of handlers) {
      if (await h.check(fakeMsg)) {
        await h.callback(fakeMsg)
      }
    }
    return fakeMsg
  }

  describe('/export command', () => {
    it('ignores non-admin users', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export', senderId: 999 })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.exportDump).not.toHaveBeenCalled()
    })

    it('ignores invocation outside private user chat', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export', chatType: 'supergroup' })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.exportDump).not.toHaveBeenCalled()
    })

    it('successfully generates and uploads database dump to admin', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export' })

      expect(mockDumpService.exportDump).toHaveBeenCalled()
      expect(fakeTg.sendMedia).toHaveBeenCalled()
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
    })
  })

  describe('/import command', () => {
    it('ignores non-admin users', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({ text: '/import', senderId: 999 })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.importDump).not.toHaveBeenCalled()
    })

    it('ignores invocation outside private user chat', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({ text: '/import', chatType: 'supergroup' })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.importDump).not.toHaveBeenCalled()
    })

    it('prompts admin when called without replying to a document', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({ text: '/import' })

      expect(fakeTg.sendText).toHaveBeenCalled()
      const calls = (
        fakeTg.sendText as unknown as { mock: { calls: unknown[][] } }
      ).mock.calls
      const sentText = extractSentText(calls[0]?.[1])
      expect(sentText).toContain('Please reply to a valid')
    })

    it('rejects replied message when media is not a document', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({
        text: '/import',
        replyToMessage: {
          media: { type: 'photo' },
        },
      })

      const calls = (
        fakeTg.sendText as unknown as { mock: { calls: unknown[][] } }
      ).mock.calls
      const sentText = extractSentText(calls[0]?.[1])
      expect(sentText).toContain('Please reply to a valid')
    })

    it('rejects replied document if file extension is not .sql.gz', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({
        text: '/import',
        replyToMessage: {
          media: {
            type: 'document',
            fileName: 'music.mp3',
          },
        },
      })

      const calls = (
        fakeTg.sendText as unknown as { mock: { calls: unknown[][] } }
      ).mock.calls
      const sentText = extractSentText(calls[0]?.[1])
      expect(sentText).toContain('must be a .sql.gz')
    })

    it('successfully restores database dump and reports merged counts', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({
        text: '/import',
        replyToMessage: {
          media: {
            type: 'document',
            fileName: 'alac_dump.sql.gz',
          },
        },
      })

      expect(fakeTg.downloadToFile).toHaveBeenCalled()
      expect(mockDumpService.importDump).toHaveBeenCalled()
      expect(fakeTg.sendText).toHaveBeenCalledTimes(2)

      const calls = (
        fakeTg.sendText as unknown as { mock: { calls: unknown[][] } }
      ).mock.calls
      const finalReply = extractSentText(calls[1]?.[1])
      expect(finalReply).toContain('Database Restored Successfully')
      expect(finalReply).toContain('Users Merged: 5')
      expect(finalReply).toContain('Tracks Merged: 20')
      expect(finalReply).toContain('Requests Merged: 50')
    })

    it('handles restore failure and reports transaction rollback', async () => {
      mockDumpService.importDump = mock(() =>
        Promise.reject(new Error('Syntax error at line 4')),
      )

      registerImportCommand(ctx)
      await dispatchMessage({
        text: '/import',
        replyToMessage: {
          media: {
            type: 'document',
            fileName: 'corrupt.sql.gz',
          },
        },
      })

      const calls = (
        fakeTg.sendText as unknown as { mock: { calls: unknown[][] } }
      ).mock.calls
      const errorReply = extractSentText(calls[1]?.[1])
      expect(errorReply).toContain('Database Restore Failed')
      expect(errorReply).toContain('Syntax error at line 4')
      expect(errorReply).toContain('Transaction rolled back')
    })
  })

  describe('registerBackupCommands helper', () => {
    it('registers both export and import commands', () => {
      registerBackupCommands(ctx)
      const internal = dp as unknown as DispatcherInternal
      const handlers = internal._groups.get(0)?.get('new_message') || []
      expect(handlers.length).toBe(2)
    })
  })
})
