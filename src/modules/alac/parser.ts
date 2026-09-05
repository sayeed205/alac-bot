export type AlacTargetType = 'track' | 'album' | 'playlist' | 'artist'

export interface ParsedTargetItem {
  id: string
  type: AlacTargetType
  storefront?: string
}

export interface ParsedAlacInput {
  items: ParsedTargetItem[]
  trackId: string
  force: boolean
  isAlbum?: boolean
  isPlaylist?: boolean
  isArtist?: boolean
  storefront?: string
}

const SONG_WITH_ALBUM_RE =
  /music\.apple\.com\/(?:([a-z]{2})\/)?album\/(?:[^/]+\/)?\d+\?i=(\d+)/i
const SONG_DIRECT_RE =
  /music\.apple\.com\/(?:([a-z]{2})\/)?song\/(?:[^/]+\/)?(\d+)/i
const ALBUM_RE = /music\.apple\.com\/(?:([a-z]{2})\/)?album\/(?:[^/]+\/)?(\d+)/i
const PLAYLIST_RE =
  /music\.apple\.com\/(?:([a-z]{2})\/)?playlist\/(?:[^/]+\/)?(pl\.(?:u-[a-zA-Z0-9]+|[a-zA-Z0-9]+))/i
const ARTIST_RE =
  /(?:music|itunes)\.apple\.com\/(?:([a-z]{2})\/)?artist\/(?:[^/]+\/)?(\d+)/i
const BARE_ID_RE = /^\d+$/
const BARE_PLAYLIST_ID_RE = /^(pl\.(?:u-[a-zA-Z0-9]+|[a-zA-Z0-9]+))$/i
const BARE_ARTIST_ID_RE = /^artist[:/](\d+)$/i

export function parseSingleItem(rawToken: string): ParsedTargetItem | null {
  const token = rawToken.trim()
  if (!token) return null

  const playlistMatch = token.match(PLAYLIST_RE)
  if (playlistMatch?.[2]) {
    return {
      id: playlistMatch[2],
      type: 'playlist',
      storefront: playlistMatch[1]?.toLowerCase(),
    }
  }

  const barePlaylistMatch = token.match(BARE_PLAYLIST_ID_RE)
  if (barePlaylistMatch?.[1]) {
    return {
      id: barePlaylistMatch[1],
      type: 'playlist',
    }
  }

  const artistMatch = token.match(ARTIST_RE)
  if (artistMatch?.[2]) {
    return {
      id: artistMatch[2],
      type: 'artist',
      storefront: artistMatch[1]?.toLowerCase(),
    }
  }

  const bareArtistMatch = token.match(BARE_ARTIST_ID_RE)
  if (bareArtistMatch?.[1]) {
    return {
      id: bareArtistMatch[1],
      type: 'artist',
    }
  }

  const songWithAlbumMatch = token.match(SONG_WITH_ALBUM_RE)
  if (songWithAlbumMatch?.[2]) {
    return {
      id: songWithAlbumMatch[2],
      type: 'track',
      storefront: songWithAlbumMatch[1]?.toLowerCase(),
    }
  }

  const songDirectMatch = token.match(SONG_DIRECT_RE)
  if (songDirectMatch?.[2]) {
    return {
      id: songDirectMatch[2],
      type: 'track',
      storefront: songDirectMatch[1]?.toLowerCase(),
    }
  }

  const albumMatch = token.match(ALBUM_RE)
  if (albumMatch?.[2]) {
    return {
      id: albumMatch[2],
      type: 'album',
      storefront: albumMatch[1]?.toLowerCase(),
    }
  }

  if (BARE_ID_RE.test(token)) {
    return {
      id: token,
      type: 'track',
    }
  }

  return null
}

export function extractBatchItems(content: string): ParsedTargetItem[] {
  const lines = content.split(/[\r\n]+/)
  const results: ParsedTargetItem[] = []
  const seen = new Set<string>()

  for (const line of lines) {
    const tokens = line.trim().split(/\s+/)
    for (const t of tokens) {
      if (t.startsWith('#') || t.startsWith('//')) break
      const item = parseSingleItem(t)
      if (item && !seen.has(`${item.type}:${item.id}`)) {
        seen.add(`${item.type}:${item.id}`)
        results.push(item)
      }
    }
  }

  return results
}

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

  const items: ParsedTargetItem[] = []
  const seen = new Set<string>()

  for (const tok of filteredTokens) {
    const item = parseSingleItem(tok)
    if (item && !seen.has(`${item.type}:${item.id}`)) {
      seen.add(`${item.type}:${item.id}`)
      items.push(item)
    }
  }

  if (items.length === 0 && replyText) {
    const replyTokens = replyText.trim().split(/\s+/)
    for (const t of replyTokens) {
      const item = parseSingleItem(t)
      if (item && !seen.has(`${item.type}:${item.id}`)) {
        seen.add(`${item.type}:${item.id}`)
        items.push(item)
      }
    }
  }

  if (items.length === 0) {
    return null
  }

  const first = items[0]
  if (!first) return null

  const res: ParsedAlacInput = {
    items,
    trackId: first.id,
    force,
    isAlbum: first.type === 'album',
    isPlaylist: first.type === 'playlist',
    isArtist: first.type === 'artist',
  }

  if (first.storefront) {
    res.storefront = first.storefront
  }

  return res
}
