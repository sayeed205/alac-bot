import fs from 'node:fs'
import path from 'node:path'

import { filters } from '@mtcute/dispatcher'

import { info, infoSpan } from '@/utils/logger.ts'
import { formatBytes } from '@/utils/progress.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerCleanCommand(ctx: CommandContext): void {
  const { dp, auth } = ctx

  dp.onNewMessage(filters.command('clean'), async (msg) => {
    using _cleanSpan = infoSpan('clean').enter()

    if (!auth.isAdmin(msg.sender.id)) {
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> This command is restricted to the bot owner.',
        ),
      )
      return
    }

    const downloadsDir = path.resolve('bot-data/downloads')
    if (!fs.existsSync(downloadsDir)) {
      await msg.replyText(
        parseDynamicHtml('✨ <b>Clean:</b> Downloads directory is empty.'),
      )
      return
    }

    let filesRemoved = 0
    let bytesFreed = 0

    try {
      const entries = fs.readdirSync(downloadsDir)
      for (const entry of entries) {
        if (entry === '.gitkeep' || entry === '.gitignore') continue
        const fullPath = path.join(downloadsDir, entry)
        try {
          const stat = fs.statSync(fullPath)
          if (stat.isFile()) {
            bytesFreed += stat.size
            fs.unlinkSync(fullPath)
            filesRemoved++
          }
        } catch {}
      }

      info('Cleaned temporary downloads directory', {
        files: filesRemoved,
        bytes: bytesFreed,
        user: msg.sender.id,
      })

      await msg.replyText(
        parseDynamicHtml(
          '🧹 <b>Temporary Storage Cleaned</b><br/><br/>' +
            `<blockquote>• Files Removed: <code>${filesRemoved}</code><br/>` +
            `• Space Reclaimed: <code>${formatBytes(bytesFreed)}</code><br/>` +
            '• Target: <code>bot-data/downloads/</code></blockquote>',
        ),
      )
    } catch (err) {
      await msg.replyText(
        parseDynamicHtml(
          `⚠️ <b>Failed to clean directory:</b> <code>${String(err)}</code>`,
        ),
      )
    }
  })
}
