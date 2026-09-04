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

    dp = Dispatcher.for(fakeTg)

    mockAuth = {
      isAdmin: mock((userId: number) => userId === 1),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    mockDumpService = {
      exportDump: mock(() =>
        Promise.resolve({
          buffer: Bun.gzipSync(Buffer.from('-- TEST DUMP')),
          filename: 'alac_dump_test.sql.gz',
          stats: {
            usersCount: 5,
            tracksCount: 20,
            requestsCount: 50,
            bytes: 1200,
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

    ctx = {
      dp,
      tg: fakeTg,
      service: {} as unknown as IAlacService,
      queue: {} as unknown as IRipQueue,
      ripper: {} as unknown as ITrackRipper,
      auth: mockAuth,
      dumpService: mockDumpService,
    }
  })

  async function dispatchMessage(options: {
    text: string
    userId?: number
    chatType?: 'user' | 'group' | 'supergroup'
    replyToMessage?: unknown
  }) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const msg = {
      _name: 'new_message',
      id: 50,
      text: options.text,
      sender: { id: options.userId ?? 1 },
      chat: { id: 100, type: options.chatType ?? 'user' },
      getReplyTo: mock(() => Promise.resolve(options.replyToMessage ?? null)),
    }

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return msg
  }

  describe('/export command', () => {
    it('ignores non-admin users', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export', userId: 999 })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.exportDump).not.toHaveBeenCalled()
    })

    it('ignores invocation outside private user chat', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export', chatType: 'group' })
      expect(fakeTg.sendText).not.toHaveBeenCalled()
      expect(mockDumpService.exportDump).not.toHaveBeenCalled()
    })

    it('successfully generates and uploads database dump to admin', async () => {
      registerExportCommand(ctx)
      await dispatchMessage({ text: '/export' })

      expect(fakeTg.sendText).toHaveBeenCalled()
      expect(mockDumpService.exportDump).toHaveBeenCalled()
      expect(fakeTg.sendMedia).toHaveBeenCalled()
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
    })
  })

  describe('/import command', () => {
    it('ignores non-admin users', async () => {
      registerImportCommand(ctx)
      await dispatchMessage({ text: '/import', userId: 999 })
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
      const sentText = String(calls[0]?.[1])
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
      const sentText = String(calls[0]?.[1])
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
      const sentText = String(calls[0]?.[1])
      expect(sentText).toContain('must be a <code>.sql.gz</code>')
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
      const finalReply =
        typeof calls[1]?.[1] === 'object' &&
        calls[1]?.[1] !== null &&
        'text' in calls[1][1]
          ? String((calls[1][1] as { text: string }).text)
          : String(calls[1]?.[1])
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
      const errorReply =
        typeof calls[1]?.[1] === 'object' &&
        calls[1]?.[1] !== null &&
        'text' in calls[1][1]
          ? String((calls[1][1] as { text: string }).text)
          : String(calls[1]?.[1])
      expect(errorReply).toContain('Database Restore Failed')
      expect(errorReply).toContain('Syntax error at line 4')
      expect(errorReply).toContain('Transaction rolled back')
    })
  })

  describe('registerBackupCommands helper', () => {
    it('registers both export and import commands', async () => {
      registerBackupCommands(ctx)
      await dispatchMessage({ text: '/export' })
      expect(mockDumpService.exportDump).toHaveBeenCalled()

      await dispatchMessage({
        text: '/import',
        replyToMessage: {
          media: {
            type: 'document',
            fileName: 'backup.sql.gz',
          },
        },
      })
      expect(mockDumpService.importDump).toHaveBeenCalled()
    })
  })
})
