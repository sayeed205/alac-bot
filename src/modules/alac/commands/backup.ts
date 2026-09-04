import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { filters } from '@mtcute/dispatcher'

import { dbDumpService as defaultDumpService } from '@/db/dump.ts'
import { debug } from '@/utils/logger.ts'

import { type CommandContext, parseDynamicHtml } from './types.ts'

export function registerExportCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx
  const dumpService = ctx.dumpService ?? defaultDumpService

  dp.onNewMessage(filters.command('export'), async (msg) => {
    if (!auth.isAdmin(msg.sender.id)) {
      debug('Non-admin attempted /export command', { user_id: msg.sender.id })
      return
    }

    if (msg.chat.type !== 'user') {
      debug('Export command attempted outside private DM', {
        chat_id: msg.chat.id,
      })
      return
    }

    const statusMsg = await tg.sendText(
      msg.chat.id,
      '📦 Generating database export...',
      { replyTo: msg.id },
    )

    const { buffer, filename, stats } = await dumpService.exportDump()
    const tmpFilePath = path.join(os.tmpdir(), filename)
    await Bun.write(tmpFilePath, buffer)

    try {
      const caption =
        '<b>📦 Database Dump Exported</b>\n\n' +
        `• <b>Users:</b> ${stats.usersCount}\n` +
        `• <b>Tracks:</b> ${stats.tracksCount}\n` +
        `• <b>Requests:</b> ${stats.requestsCount}\n` +
        `• <b>Compressed Size:</b> ${(stats.bytes / 1024).toFixed(1)} KB`

      await tg.sendMedia(msg.chat.id, {
        type: 'document',
        file: Bun.file(tmpFilePath),
        fileName: filename,
        caption: parseDynamicHtml(caption),
      })
    } finally {
      try {
        await fs.promises.unlink(tmpFilePath)
      } catch {}
      try {
        await tg.deleteMessagesById(msg.chat.id, [statusMsg.id])
      } catch {}
    }
  })
}

export function registerImportCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx
  const dumpService = ctx.dumpService ?? defaultDumpService

  dp.onNewMessage(filters.command('import'), async (msg) => {
    if (!auth.isAdmin(msg.sender.id)) {
      debug('Non-admin attempted /import command', { user_id: msg.sender.id })
      return
    }

    if (msg.chat.type !== 'user') {
      debug('Import command attempted outside private DM', {
        chat_id: msg.chat.id,
      })
      return
    }

    const reply = await msg.getReplyTo().catch(() => null)
    if (reply?.media?.type !== 'document') {
      await tg.sendText(
        msg.chat.id,
        '⚠️ Please reply to a valid <code>.sql.gz</code> database dump document with <code>/import</code> to restore.',
        { replyTo: msg.id },
      )
      return
    }

    const doc = reply.media
    const fileName =
      'fileName' in doc && typeof doc.fileName === 'string'
        ? doc.fileName
        : 'name' in doc && typeof (doc as { name?: unknown }).name === 'string'
          ? (doc as { name: string }).name
          : ''
    if (!fileName.endsWith('.sql.gz') && !fileName.endsWith('.gz')) {
      await tg.sendText(
        msg.chat.id,
        '⚠️ The replied file must be a <code>.sql.gz</code> database dump.',
        { replyTo: msg.id },
      )
      return
    }

    const statusMsg = await tg.sendText(
      msg.chat.id,
      '⏳ Downloading and restoring database dump...',
      { replyTo: msg.id },
    )
    const tmpFile = path.join(os.tmpdir(), `import_${Date.now()}.sql.gz`)

    try {
      await tg.downloadToFile(tmpFile, doc)
      const fileBytes = new Uint8Array(await Bun.file(tmpFile).arrayBuffer())
      const restoreStats = await dumpService.importDump(fileBytes)

      const resultText =
        '<b>✅ Database Restored Successfully!</b>\n\n' +
        `• <b>Users Merged:</b> ${restoreStats.usersMerged}\n` +
        `• <b>Tracks Merged:</b> ${restoreStats.tracksMerged}\n` +
        `• <b>Requests Merged:</b> ${restoreStats.requestsMerged}\n` +
        `• <b>Elapsed Time:</b> ${restoreStats.durationMs}ms`

      await tg.sendText(msg.chat.id, parseDynamicHtml(resultText), {
        replyTo: msg.id,
      })
    } catch (err: unknown) {
      const errMsg = err instanceof Error ? err.message : String(err)
      await tg.sendText(
        msg.chat.id,
        parseDynamicHtml(
          `<b>❌ Database Restore Failed</b>\n\n<code>${errMsg}</code>\n\n<i>Transaction rolled back. Database state remains unchanged.</i>`,
        ),
        { replyTo: msg.id },
      )
    } finally {
      try {
        await fs.promises.unlink(tmpFile)
      } catch {}
      try {
        await tg.deleteMessagesById(msg.chat.id, [statusMsg.id])
      } catch {}
    }
  })
}

export function registerBackupCommands(ctx: CommandContext): void {
  registerExportCommand(ctx)
  registerImportCommand(ctx)
}
