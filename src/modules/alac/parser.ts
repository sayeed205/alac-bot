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

  // Remove command if present (/alac or /rerip)
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

  // If no candidate in command text, check reply text
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

  // 1. Song query param in album link: ?i=67890
  const songWithAlbumMatch = candidate.match(SONG_WITH_ALBUM_RE)
  if (songWithAlbumMatch?.[1]) {
    return { trackId: songWithAlbumMatch[1], force, isAlbum: false }
  }

  // 2. Direct song link: /song/12345
  const songDirectMatch = candidate.match(SONG_DIRECT_RE)
  if (songDirectMatch?.[1]) {
    return { trackId: songDirectMatch[1], force, isAlbum: false }
  }

  // 3. Album link: /album/12345
  const albumMatch = candidate.match(ALBUM_RE)
  if (albumMatch?.[1]) {
    return { trackId: albumMatch[1], force, isAlbum: true }
  }

  // 4. Raw numeric ID
  if (BARE_ID_RE.test(candidate)) {
    return { trackId: candidate, force, isAlbum: false }
  }

  return null
}
