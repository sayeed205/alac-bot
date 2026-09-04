export interface ParsedAlacInput {
  trackId: string
  force: boolean
  isAlbum?: boolean
}

const SONG_WITH_ALBUM_RE =
  /music\.apple\.com\/(?:[a-z]{2}\/)?album\/(?:[^/]+\/)?\d+\?i=(\d+)/i
const SONG_DIRECT_RE =
  /music\.apple\.com\/(?:[a-z]{2}\/)?song\/(?:[^/]+\/)?(\d+)/i
const ALBUM_RE = /music\.apple\.com\/(?:[a-z]{2}\/)?album\/(?:[^/]+\/)?(\d+)/i
const BARE_ID_RE = /^\d+$/

export function parseAlacInput(
  rawText: string,
  replyText?: string,
): ParsedAlacInput | null {
  const text = rawText.trim()
  const tokens = text.split(/\s+/)

  if (tokens[0]?.startsWith('/')) {
    tokens.shift()
  }

  let force = false
  const filteredTokens: string[] = []

  for (const token of tokens) {
    if (token === '-f' || token === '--force') {
      force = true
    } else if (token) {
      filteredTokens.push(token)
    }
  }

  let candidate = filteredTokens[0]

  if (!candidate && replyText) {
    const replyTokens = replyText.trim().split(/\s+/)
    for (const t of replyTokens) {
      if (
        SONG_WITH_ALBUM_RE.test(t) ||
        SONG_DIRECT_RE.test(t) ||
        ALBUM_RE.test(t) ||
        BARE_ID_RE.test(t)
      ) {
        candidate = t
        break
      }
    }
  }

  if (!candidate) {
    return null
  }

  const songWithAlbumMatch = candidate.match(SONG_WITH_ALBUM_RE)
  if (songWithAlbumMatch?.[1]) {
    return { trackId: songWithAlbumMatch[1], force, isAlbum: false }
  }

  const songDirectMatch = candidate.match(SONG_DIRECT_RE)
  if (songDirectMatch?.[1]) {
    return { trackId: songDirectMatch[1], force, isAlbum: false }
  }

  const albumMatch = candidate.match(ALBUM_RE)
  if (albumMatch?.[1]) {
    return { trackId: albumMatch[1], force, isAlbum: true }
  }

  if (BARE_ID_RE.test(candidate)) {
    return { trackId: candidate, force, isAlbum: false }
  }

  return null
}
