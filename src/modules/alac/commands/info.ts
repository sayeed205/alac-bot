import { html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { infoSpan } from '@/utils/logger.ts'

import { fetchTrackMeta } from '../itunes.ts'
import { parseAlacInput } from '../parser.ts'
import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

function formatDurationSec(sec: number): string {
  const m = Math.floor(sec / 60)
  const s = String(sec % 60).padStart(2, '0')
  return `${m}:${s}`
}

export function registerInfoCommand(ctx: CommandContext): void {
  const { dp, service, auth } = ctx

  dp.onNewMessage(filters.command('info'), async (msg) => {
    using _infoSpan = infoSpan('info').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const replyMsg = await msg.getReplyTo().catch(() => null)
    const parsed = parseAlacInput(msg.text, replyMsg?.text)

    if (!parsed) {
      await msg.replyText(
        parseDynamicHtml(
          'ℹ️ <b>Track Info Usage:</b><br/><br/>' +
            '<blockquote>• <code>/info &lt;apple_music_link | id&gt;</code><br/>' +
            '• Reply to an Apple Music link with <code>/info</code></blockquote>',
        ),
      )
      return
    }

    const trackId = parsed.trackId

    try {
      const meta = await fetchTrackMeta(trackId)
      const cached = await service.findCachedTrack(trackId)

      const title = html.escape(meta.title)
      const artist = html.escape(meta.artist)
      const album = html.escape(meta.album)
      const duration = formatDurationSec(meta.duration)
      const year = meta.releaseDate ? meta.releaseDate.slice(0, 4) : 'Unknown'
      const genre = html.escape(meta.primaryGenre || 'Music')

      let cacheStatus = '❌ <b>Not Cached</b>'
      let cacheDetails = `Use <code>/alac ${trackId}</code> to rip in lossless ALAC.`

      if (cached) {
        const qualityParts: string[] = ['ALAC']
        if (cached.bitDepth) qualityParts.push(`${cached.bitDepth}-bit`)
        if (cached.sampleRate) {
          qualityParts.push(`${(cached.sampleRate / 1000).toFixed(1)} kHz`)
        }
        cacheStatus = '✅ <b>Cached in Database</b>'
        cacheDetails = `Quality: <code>${qualityParts.join(' • ')}</code><br/>Dump Message: <code>#${cached.messageId}</code>`
      }

      const card =
        `🎵 <b>${title}</b> — ${artist}<br/><br/>` +
        `<blockquote><b>Metadata:</b><br/>` +
        `• Album: <b>${album}</b><br/>` +
        `• Track Number: <code>${meta.trackNumber || 1}</code> of <code>${meta.trackCount || 1}</code><br/>` +
        `• Duration: <code>${duration}</code><br/>` +
        `• Release Year: <code>${year}</code><br/>` +
        `• Genre: <code>${genre}</code><br/>` +
        `• Apple Track ID: <code>${trackId}</code></blockquote><br/>` +
        `<blockquote><b>Cache Status:</b><br/>` +
        `• Status: ${cacheStatus}<br/>` +
        `• ${cacheDetails}</blockquote>`

      await msg.replyText(parseDynamicHtml(card))
    } catch (err) {
      await msg.replyText(
        parseDynamicHtml(
          `⚠️ <b>Failed to fetch info:</b> <code>${html.escape(String(err))}</code>`,
        ),
      )
    }
  })
}
