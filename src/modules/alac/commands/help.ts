import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerHelpCommand(ctx: CommandContext): void {
  const { dp, auth } = ctx

  dp.onNewMessage(filters.command('help'), async (msg) => {
    using _helpSpan = infoSpan('help').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const isAdmin = auth.isAdmin(msg.sender.id)

    let helpText =
      '📖 <b>ALAC Bot Usage Guide</b><br/><br/>' +
      '<blockquote><b>General Commands:</b><br/>' +
      '• <code>/alac &lt;link | id&gt;</code> — Rip Apple Music track/album in lossless ALAC<br/>' +
      '• <code>/search &lt;query&gt;</code> — Search cached tracks for instant download<br/>' +
      '• <code>/info &lt;link | id&gt;</code> — Inspect track details & check cache status<br/>' +
      '• <code>/queue</code> — Check active & pending rip jobs<br/>' +
      '• <code>/ping</code> — Test bot latency & system health<br/>' +
      '• <code>/help</code> — Show this usage guide</blockquote><br/>' +
      '<blockquote expandable>💡 <b>Ripping Tips:</b><br/>' +
      '• Reply to an Apple Music link with <code>/alac</code> to rip it.<br/>' +
      '• Supported: song links, album links with <code>?i=...</code>, direct album links, bare song IDs.<br/>' +
      '• Synchronized lyrics (Enhanced LRC) and cover artwork are automatically embedded.</blockquote>'

    if (isAdmin) {
      helpText +=
        '<br/><br/><blockquote><b>Admin Commands:</b><br/>' +
        '• <code>/alac &lt;link&gt; -f</code> or <code>/rerip</code> — Force re-rip, bypassing cache<br/>' +
        '• <code>/delete &lt;track_id&gt;</code> — Delete track from DB & dump channel<br/>' +
        '• <code>/clean</code> — Delete leftover temporary download files<br/>' +
        '• <code>/auth &lt;id | @username&gt;</code> — Authorize a user or group<br/>' +
        '• <code>/revoke &lt;id | @username&gt;</code> — Revoke authorization<br/>' +
        '• <code>/authlist [page]</code> — List authorized users and groups<br/>' +
        '• <code>/stats</code> — View caching & rip analytics<br/>' +
        '• <code>/index</code> — Sync database with dump channel & prune deleted tracks</blockquote>'
    }

    await msg.replyText(parseDynamicHtml(helpText))
  })
}
