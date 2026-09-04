export enum LyricsTier {
  WORD_SYNCED = 1000,
  LINE_SYNCED = 500,
  PLAIN = 100,
  NONE = 0,
}

export interface LyricsCandidate {
  text: string
  provider: string
  tier: LyricsTier
  score: number
}

const WORD_SYNC_REGEX = /<\d{1,3}:\d{2}(?:[.:]\d{2,3})?>/
const LINE_SYNC_REGEX = /\[\d{1,3}:\d{2}(?:[.:]\d{2,3})?\]/

function formatTimestamp(timeStr: string): string {
  let totalSec = 0
  if (timeStr.includes(':')) {
    const parts = timeStr.split(':')
    const minPart = parts[0] ? Number.parseFloat(parts[0]) : 0
    const secPart = parts[1] ? Number.parseFloat(parts[1]) : 0
    totalSec = minPart * 60 + secPart
  } else {
    totalSec = Number.parseFloat(timeStr)
  }

  if (Number.isNaN(totalSec)) totalSec = 0

  const min = Math.floor(totalSec / 60)
  const sec = Math.floor(totalSec % 60)
  const ms = Math.floor(Math.round((totalSec % 1) * 1000))
  return `${String(min).padStart(2, '0')}:${String(sec).padStart(2, '0')}.${String(ms).padStart(3, '0')}`
}

/**
 * Converts Apple Music TTML with word-level spans into Enhanced LRC (ELRC) format.
 */
export function convertTtmlToElrc(ttml: string): string | null {
  if (!ttml.includes('<span') || !ttml.includes('begin=')) {
    return null
  }

  const pRegex = /<p\b[^>]*\bbegin=["']([^"']+)["'][^>]*>([\s\S]*?)<\/p>/gi
  const spanRegex =
    /<span\b[^>]*\bbegin=["']([^"']+)["'][^>]*>([\s\S]*?)<\/span>/gi

  const lines: string[] = []
  let pMatch: RegExpExecArray | null = pRegex.exec(ttml)

  while (pMatch !== null) {
    const lineStart = formatTimestamp(pMatch[1] ?? '0')
    const inner = pMatch[2] ?? ''

    let elrcLine = `[${lineStart}]`
    let spanMatch: RegExpExecArray | null = spanRegex.exec(inner)
    let wordCount = 0

    while (spanMatch !== null) {
      wordCount++
      const wordStart = formatTimestamp(spanMatch[1] ?? '0')
      const wordText = (spanMatch[2] ?? '')
        .replace(/<[^>]+>/g, '')
        .replace(/&amp;/g, '&')
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
      elrcLine += `<${wordStart}>${wordText} `
      spanMatch = spanRegex.exec(inner)
    }

    if (wordCount > 0) {
      lines.push(elrcLine.trimEnd())
    } else {
      const cleanLine = inner.replace(/<[^>]+>/g, '').trim()
      if (cleanLine.length > 0) {
        lines.push(`[${lineStart}]${cleanLine}`)
      }
    }

    pMatch = pRegex.exec(ttml)
  }

  return lines.length > 0 ? lines.join('\n') : null
}

export function detectLyricsTier(lyrics: string): LyricsTier {
  const trimmed = lyrics.trim()
  if (!trimmed || trimmed.length < 10) return LyricsTier.NONE

  // Count lines with meaningful content
  const lines = trimmed.split('\n').filter((l) => l.trim().length > 0)
  if (lines.length < 2) return LyricsTier.NONE

  const hasWordSync = lines.some((l) => WORD_SYNC_REGEX.test(l))
  if (hasWordSync) return LyricsTier.WORD_SYNCED

  const hasLineSync = lines.some((l) => LINE_SYNC_REGEX.test(l))
  if (hasLineSync) return LyricsTier.LINE_SYNCED

  return LyricsTier.PLAIN
}

export function scoreLyrics(
  text: string,
  provider: string,
  providerWeight = 0,
): LyricsCandidate {
  const tier = detectLyricsTier(text)
  let score = 0

  if (tier !== LyricsTier.NONE) {
    score = tier + providerWeight
  }

  return {
    text: text.trim(),
    provider,
    tier,
    score,
  }
}

// ----------------------------------------------------------------------
// Providers
// ----------------------------------------------------------------------

const USER_AGENT = 'AlacBot/1.0'

async function fetchPaxsenix(trackId: string): Promise<LyricsCandidate[]> {
  try {
    const url = `https://lyrics.paxsenix.org/apple-music/lyrics?id=${encodeURIComponent(trackId)}`
    const resp = await fetch(url, {
      headers: { 'User-Agent': USER_AGENT },
      signal: AbortSignal.timeout(5000),
    })

    if (!resp.ok) return []

    const data = (await resp.json()) as {
      elrc?: string | null
      lrc?: string | null
      plain?: string | null
      ttmlContent?: string | null
    }

    const candidates: LyricsCandidate[] = []

    // Check direct elrc
    if (data.elrc?.trim()) {
      candidates.push(scoreLyrics(data.elrc, 'Paxsenix Apple Music (ELRC)', 30))
    }

    // Check TTML conversion to word-synced ELRC
    if (data.ttmlContent?.trim()) {
      const converted = convertTtmlToElrc(data.ttmlContent)
      if (converted) {
        candidates.push(
          scoreLyrics(converted, 'Paxsenix Apple Music (TTML-ELRC)', 30),
        )
      }
    }

    // Check line-synced lrc
    if (data.lrc?.trim()) {
      candidates.push(scoreLyrics(data.lrc, 'Paxsenix Apple Music (LRC)', 25))
    }

    // Check plain
    if (data.plain?.trim()) {
      candidates.push(
        scoreLyrics(data.plain, 'Paxsenix Apple Music (Plain)', 20),
      )
    }

    return candidates
  } catch {
    return []
  }
}

async function fetchBetterLyrics(
  title: string,
  artist: string,
): Promise<LyricsCandidate[]> {
  try {
    const url = `https://lyrics-api.boidu.dev/getLyrics?s=${encodeURIComponent(title)}&a=${encodeURIComponent(artist)}`
    const resp = await fetch(url, {
      headers: { 'User-Agent': USER_AGENT },
      signal: AbortSignal.timeout(5000),
    })

    if (!resp.ok) return []

    const data = (await resp.json()) as {
      ttml?: string | null
      lrc?: string | null
    }

    const candidates: LyricsCandidate[] = []

    if (data.ttml?.trim()) {
      const converted = convertTtmlToElrc(data.ttml)
      if (converted) {
        candidates.push(
          scoreLyrics(converted, 'BetterLyrics (Word Synced)', 25),
        )
      }
    }

    if (data.lrc?.trim()) {
      candidates.push(scoreLyrics(data.lrc, 'BetterLyrics (LRC)', 20))
    }

    return candidates
  } catch {
    return []
  }
}

async function fetchLrcLibExact(
  title: string,
  artist: string,
  album?: string,
  duration?: number,
): Promise<LyricsCandidate[]> {
  try {
    const params = new URLSearchParams({
      artist_name: artist,
      track_name: title,
    })
    if (album) params.set('album_name', album)
    if (duration) params.set('duration', String(duration))

    const resp = await fetch(`https://lrclib.net/api/get?${params}`, {
      headers: { 'User-Agent': USER_AGENT },
      signal: AbortSignal.timeout(5000),
    })

    if (!resp.ok) return []

    const data = (await resp.json()) as {
      syncedLyrics?: string | null
      plainLyrics?: string | null
    }

    const candidates: LyricsCandidate[] = []

    if (data.syncedLyrics?.trim()) {
      candidates.push(scoreLyrics(data.syncedLyrics, 'LRCLIB Exact (LRC)', 15))
    }

    if (data.plainLyrics?.trim()) {
      candidates.push(scoreLyrics(data.plainLyrics, 'LRCLIB Exact (Plain)', 10))
    }

    return candidates
  } catch {
    return []
  }
}

async function fetchLrcLibSearch(
  title: string,
  artist: string,
  targetDuration?: number,
): Promise<LyricsCandidate[]> {
  try {
    const params = new URLSearchParams({
      q: `${title} ${artist}`,
    })

    const resp = await fetch(`https://lrclib.net/api/search?${params}`, {
      headers: { 'User-Agent': USER_AGENT },
      signal: AbortSignal.timeout(5000),
    })

    if (!resp.ok) return []

    const results = (await resp.json()) as Array<{
      syncedLyrics?: string | null
      plainLyrics?: string | null
      duration?: number
    }>

    if (!Array.isArray(results) || results.length === 0) return []

    // Sort results by closest duration match if target duration is available
    const sorted = [...results].sort((a, b) => {
      if (!targetDuration) return 0
      const diffA = Math.abs((a.duration ?? 0) - targetDuration)
      const diffB = Math.abs((b.duration ?? 0) - targetDuration)
      return diffA - diffB
    })

    const best = sorted[0]
    if (!best) return []

    const candidates: LyricsCandidate[] = []

    if (best.syncedLyrics?.trim()) {
      candidates.push(scoreLyrics(best.syncedLyrics, 'LRCLIB Search (LRC)', 5))
    }

    if (best.plainLyrics?.trim()) {
      candidates.push(scoreLyrics(best.plainLyrics, 'LRCLIB Search (Plain)', 0))
    }

    return candidates
  } catch {
    return []
  }
}

// ----------------------------------------------------------------------
// Main Ranked Fetcher
// ----------------------------------------------------------------------

export async function fetchLyrics(
  trackId: string,
  meta: {
    title: string
    artist: string
    album?: string
    duration?: number
  },
): Promise<string | null> {
  // Query all providers concurrently
  const settled = await Promise.allSettled([
    fetchPaxsenix(trackId),
    fetchBetterLyrics(meta.title, meta.artist),
    fetchLrcLibExact(meta.title, meta.artist, meta.album, meta.duration),
    fetchLrcLibSearch(meta.title, meta.artist, meta.duration),
  ])

  const allCandidates: LyricsCandidate[] = []

  for (const outcome of settled) {
    if (outcome.status === 'fulfilled') {
      allCandidates.push(...outcome.value)
    }
  }

  // Filter out invalid/empty candidates
  const validCandidates = allCandidates.filter(
    (c) => c.tier !== LyricsTier.NONE && c.score > 0,
  )

  if (validCandidates.length === 0) {
    return null
  }

  // Sort descending by score (Tier 1 word-by-word always dominates Tier 2 and Tier 3)
  validCandidates.sort((a, b) => b.score - a.score)

  const topPick = validCandidates[0]
  return topPick ? topPick.text : null
}
