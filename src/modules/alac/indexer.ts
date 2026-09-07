import type { TelegramClient } from '@mtcute/bun'
import { html } from '@mtcute/bun'

import type { FormattedString } from '@/utils/html.ts'
import { parseDynamicHtml } from '@/utils/html.ts'

import type { IAlacService } from './service.ts'

export interface DumpCaptionMetadata {
  appleTrackId: string
  title: string
  artist: string
  album: string
  duration: number
  bitDepth: number
  sampleRate: number
  codec?: string
  genre?: string
  releaseDate?: string
  trackNumber?: number
  trackCount?: number
}

const DUMP_PAYLOAD_REGEX = /(\{[\s\S]*?"id"\s*:\s*"[^"]+"[\s\S]*?\})/s

export interface ParsedDumpMetadata {
  appleTrackId: string
  title: string
  artist: string
  album: string
  duration: number
  bitDepth: number
  sampleRate: number
  genre: string
  releaseDate: string
  trackNumber: number
  trackCount: number
}

/**
 * Formats a clean human-readable track caption with an expandable blockquote containing
 * rich metadata and machine-readable payload.
 */
export function formatDumpCaption(meta: DumpCaptionMetadata): FormattedString {
  const parts: string[] = []
  parts.push(
    `🎵 <b>${html.escape(meta.title || '')}</b> — ${html.escape(meta.artist || '')}`,
  )
  parts.push(`💽 ${html.escape(meta.album || '')}`)

  const m = Math.floor(meta.duration / 60)
  const s = String(meta.duration % 60).padStart(2, '0')
  const specParts: string[] = [
    'ALAC',
    `${meta.bitDepth}-bit`,
    `${(meta.sampleRate / 1000).toFixed(1)} kHz`,
    `${m}:${s}`,
  ]

  parts.push(`🎧 ${specParts.join(' • ')}`)

  const payload = {
    id: meta.appleTrackId,
    title: meta.title,
    artist: meta.artist,
    album: meta.album,
    dur: meta.duration,
    bit: meta.bitDepth,
    hz: meta.sampleRate,
    genre: meta.genre || 'Music',
    date: meta.releaseDate || '',
    trk: meta.trackNumber ?? 1,
    cnt: meta.trackCount ?? 1,
  }

  const payloadJson = JSON.stringify(payload, null, 2)
    .split('\n')
    .map((line) => html.escape(line))
    .join('<br/>')

  const quoteLines: string[] = [
    '<b>Track Specs & Metadata:</b>',
    `• Quality: <b>ALAC ${meta.bitDepth}-bit / ${(meta.sampleRate / 1000).toFixed(1)} kHz</b>`,
    `• Album: <b>${html.escape(meta.album || '')}</b>`,
    `• Track: <b>${meta.trackNumber ?? 1}/${meta.trackCount ?? 1}</b>`,
    `• Genre: <b>${html.escape(meta.genre || 'Music')}</b>`,
    `• Release Date: <b>${html.escape(meta.releaseDate || '')}</b>`,
    `• Apple Track ID: <b>${html.escape(meta.appleTrackId || '')}</b>`,
    '',
    payloadJson,
  ]

  parts.push(`<blockquote expandable>${quoteLines.join('<br/>')}</blockquote>`)

  return parseDynamicHtml(parts.join('<br/>'))
}

/**
 * Extracts the structured metadata from message text / caption.
 */
export function parseDumpCaption(
  text: string | null | undefined,
): ParsedDumpMetadata | null {
  if (!text) return null
  const match = text.match(DUMP_PAYLOAD_REGEX)
  if (!match?.[1]) return null

  try {
    const parsed = JSON.parse(match[1])
    if (!parsed.id || typeof parsed.id !== 'string') return null

    return {
      appleTrackId: parsed.id,
      title: String(parsed.title ?? ''),
      artist: String(parsed.artist ?? ''),
      album: String(parsed.album ?? ''),
      duration: Number(parsed.dur ?? 0),
      bitDepth: Number(parsed.bit ?? 16),
      sampleRate: Number(parsed.hz ?? 44100),
      genre: String(parsed.genre ?? 'Music'),
      releaseDate: String(parsed.date ?? ''),
      trackNumber: Number(parsed.trk ?? 1),
      trackCount: Number(parsed.cnt ?? 1),
    }
  } catch {
    return null
  }
}

export interface IndexSummary {
  scanned: number
  synced: number
  pruned: number
  skipped: number
  durationMs: number
}

/**
 * Scans all messages in the dump channel using batch getMessages, restores tracks to the database,
 * and prunes obsolete records that were deleted from Telegram.
 */
export async function indexDumpChannel(
  tg: TelegramClient,
  service: IAlacService,
  dumpChannelId: number | string,
  onProgress?: (scanned: number, synced: number) => Promise<void> | void,
): Promise<IndexSummary> {
  const startTime = Date.now()
  let scanned = 0
  let synced = 0
  let skipped = 0

  const validTrackIds = new Set<string>()

  // Telegram Bots cannot call messages.getHistory (BOT_METHOD_INVALID).
  // Instead, determine the latest channel message ID via a temporary probe,
  // then fetch all messages in chunks of 100 via tg.getMessages.
  const probe = await tg.sendText(dumpChannelId, '🔄 Indexing...')
  const maxId = probe.id
  await tg.deleteMessagesById(dumpChannelId, [maxId]).catch(() => null)

  const BATCH_SIZE = 100
  // Iterate backwards from maxId - 1 down to 1
  for (let end = maxId - 1; end >= 1; end -= BATCH_SIZE) {
    const start = Math.max(1, end - BATCH_SIZE + 1)
    const batchIds: number[] = []
    for (let id = end; id >= start; id--) {
      batchIds.push(id)
    }

    const messages = await tg.getMessages(dumpChannelId, batchIds)

    for (const message of messages) {
      if (!message) {
        // Message was deleted or doesn't exist
        continue
      }

      scanned++

      if (message.media?.type !== 'audio') {
        skipped++
        continue
      }

      const meta = parseDumpCaption(message.text)
      if (!meta) {
        skipped++
        continue
      }

      const audio = message.media
      const fileId = audio.fileId
      const fileUniqueId = audio.uniqueFileId

      await service.saveTrack({
        appleTrackId: meta.appleTrackId,
        messageId: message.id,
        fileId,
        fileUniqueId,
        title: meta.title || audio.title || 'Unknown Title',
        artist: meta.artist || audio.performer || 'Unknown Artist',
        album: meta.album || 'Unknown Album',
        duration: meta.duration || audio.duration || 0,
        bitDepth: meta.bitDepth,
        sampleRate: meta.sampleRate,
        genre: meta.genre || 'Music',
        releaseDate: meta.releaseDate || '',
        trackNumber: meta.trackNumber || 1,
        trackCount: meta.trackCount || 1,
      })

      validTrackIds.add(meta.appleTrackId)
      synced++
    }

    if (onProgress) {
      await onProgress(scanned, synced)
    }
  }

  const pruned = await service.deleteTracksNotIn(Array.from(validTrackIds))

  return {
    scanned,
    synced,
    pruned,
    skipped,
    durationMs: Date.now() - startTime,
  }
}

/**
 * Formats a completion report for the /index command.
 */
export function formatIndexSummaryHtml(summary: IndexSummary): FormattedString {
  const timeSec = (summary.durationMs / 1000).toFixed(1)
  const lines = [
    '✅ <b>Dump Channel Sync Complete</b>',
    '',
    `• <b>Messages Scanned:</b> <code>${summary.scanned}</code>`,
    `• <b>Tracks Synced:</b> <code>${summary.synced}</code>`,
    `• <b>Ghost Tracks Pruned:</b> <code>${summary.pruned}</code>`,
    `• <b>Skipped (non-tracks):</b> <code>${summary.skipped}</code>`,
    `• <b>Time Elapsed:</b> <code>${timeSec}s</code>`,
  ]
  return parseDynamicHtml(lines.join('<br/>'))
}
