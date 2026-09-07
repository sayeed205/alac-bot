import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export function registerHelpCommand(ctx: CommandContext): void {
  const { dp, auth } = ctx

  dp.onNewMessage(filters.command(['help', 'start']), async (msg) => {
    using _ = infoSpan('help').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const isAdmin = auth.isAdmin(msg.sender.id)

    let helpText =
      '📖 <b>ALAC Bot Usage Guide</b><br/><br/>' +
      '<blockquote><b>General Commands:</b><br/>' +
      '• <code>/alac &lt;link | id&gt;</code> — Rip track, album, or playlist in lossless ALAC<br/>' +
      '• <code>/batch &lt;links...&gt;</code> — Download multiple links or attach a <code>.txt</code> file<br/>' +
      '• <code>/cancel</code> — Cancel an active download task<br/>' +
      '• <code>/spec</code> — Reply to audio file to generate frequency spectrogram<br/>' +
      '• <code>/report</code> — Reply to song to report corruption or issues to admin<br/>' +
      '• <code>/search &lt;query&gt;</code> — Search cached tracks for instant download<br/>' +
      '• <code>/info &lt;link | id&gt;</code> — Inspect track details & check cache status<br/>' +
      '• <code>/queue</code> — Check active & pending rip jobs<br/>' +
      '• <code>/ping</code> — Test bot latency & system health<br/>' +
      '• <code>/help</code> — Show this usage guide</blockquote><br/>' +
      '<blockquote expandable>💡 <b>Ripping Tips:</b><br/>' +
      '• <b>Aliases:</b> <code>/rip</code>, <code>/batch</code>, <code>/dl</code>, <code>/download</code><br/>' +
      '• <b>Cancel Download:</b> Tap <code>❌ Cancel Download</code> button on the progress card or use <code>/cancel</code>.<br/>' +
      '• <b>Spectrogram:</b> Reply to any audio file with <code>/spec</code> or <code>/spectogram</code> to visually verify uncompressed lossless quality.<br/>' +
      '• <b>Reporting Issues:</b> Reply to any corrupt or cut-off track with <code>/report</code> to notify the admin for a re-rip.<br/>' +
      '• <b>Group Chats:</b> Audio files are delivered to your private DM to keep the chat clean!<br/>' +
      '• <b>Playlists:</b> Paste any Apple Music playlist link to download all songs.<br/>' +
      '• <b>Batch Files:</b> Send or reply to a <code>.txt</code> file containing links with <code>/alac</code>.<br/>' +
      '• Synchronized lyrics (Enhanced LRC) and cover artwork are automatically embedded.</blockquote>'

    if (isAdmin) {
      helpText +=
        '<br/><br/><blockquote><b>Admin Commands:</b><br/>' +
        '• <code>/settings</code> — Bot operational settings & ripping toggles<br/>' +
        '• <code>/cache &lt;link&gt;</code> — Pre-cache/seed tracks directly into dump channel without sending audio (alias: <code>/dump</code>)<br/>' +
        '• <code>/random</code> — Interactive random album discovery & dump<br/>' +
        '• <code>/alac &lt;link&gt; -f</code> or <code>/rerip</code> — Force re-rip, bypassing cache<br/>' +
        '• <code>/delete &lt;track_id&gt;</code> — Delete track from DB & dump channel<br/>' +
        '• <code>/auth &lt;user_id | reply&gt;</code> — Whitelist user<br/>' +
        '• <code>/revoke &lt;user_id | reply&gt;</code> — Revoke user access<br/>' +
        '• <code>/authlist</code> — List authorized users<br/>' +
        '• <code>/stats</code> — View bot download & cache statistics<br/>' +
        '• <code>/cleanup</code> — Reconcile DB with Telegram dump channel</blockquote>'
    }

    await msg.replyText(parseDynamicHtml(helpText))
  })
}
