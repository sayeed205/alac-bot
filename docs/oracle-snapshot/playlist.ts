import { debug, error, info, infoSpan, warn } from '@/utils/logger.ts'

export interface PlaylistTrack {
  id: string
  title: string
  artist: string
  duration?: number
}

export interface PlaylistData {
  id: string
  title: string
  curatorName?: string
  description?: string
  tracks: PlaylistTrack[]
}

let cachedToken: string | null = null
let cachedTokenExpiresAt = 0

// Static reliable backup token in case live scraping fails
const FALLBACK_TOKEN =
  'eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiIsImtpZCI6IldlYlBsYXlLaWQifQ.eyJpc3MiOiJBTVBXZWJQbGF5IiwiaWF0IjoxNzg2NjMyOTI0LCJleHAiOjE3OTI2ODA5MjQsInJvb3RfaHR0cHNfb3JpZ2luIjpbImFwcGxlLmNvbSJdfQ.hBgj61sZf-y7bmuvT-joXAUAcf7TVJ51732xnH5vFkLHOmsQHxVqGMYUuI4h8c0-RX3fRY3moylhLW8fewFJyw'

/**
 * Retrieves the Apple Music Web Client developer token, dynamically scraping it
 * from the web player asset bundle or using the cached/fallback token.
 */
export async function getAppleMusicDeveloperToken(): Promise<string> {
  const now = Date.now()
  if (cachedToken && cachedTokenExpiresAt > now) {
    return cachedToken
  }

  try {
    const browseRes = await fetch('https://music.apple.com/us/browse', {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: AbortSignal.timeout(10_000),
    })

    if (browseRes.ok) {
      const html = await browseRes.text()
      const jsMatch = html.match(/\/assets\/index~[a-zA-Z0-9]+\.js/)
      if (jsMatch?.[0]) {
        const jsRes = await fetch(`https://music.apple.com${jsMatch[0]}`, {
          signal: AbortSignal.timeout(10_000),
        })
        if (jsRes.ok) {
          const js = await jsRes.text()
          const tokenMatch = js.match(/developerToken:([$a-zA-Z0-9_]+)/)
          if (tokenMatch?.[1]) {
            const varName = tokenMatch[1]
            const valMatch = js.match(
              new RegExp(`${varName.replace('$', '\\$')}\\s*=\\s*"([^"]+)"`),
            )
            if (valMatch?.[1]) {
              cachedToken = valMatch[1]
              // Cache for 24 hours
              cachedTokenExpiresAt = now + 24 * 60 * 60 * 1000
              debug('Extracted live Apple Music developer token')
              return cachedToken
            }
          }

          const directJwt = js.match(
            /eyJh[A-Za-z0-9_-]*\.[A-Za-z0-9_-]*\.[A-Za-z0-9_-]*/,
          )
          if (directJwt?.[0]) {
            cachedToken = directJwt[0]
            cachedTokenExpiresAt = now + 24 * 60 * 60 * 1000
            return cachedToken
          }
        }
      }
    }
  } catch (err) {
    warn('Dynamic developer token extraction failed, using fallback token', {
      error: String(err),
    })
  }

  cachedToken = FALLBACK_TOKEN
  cachedTokenExpiresAt = now + 12 * 60 * 60 * 1000
  return cachedToken
}

interface RawPlaylistResponse {
  data?: Array<{
    id: string
    attributes?: {
      name?: string
      curatorName?: string
      description?: { standard?: string }
    }
    relationships?: {
      tracks?: {
        href?: string
        next?: string
        data?: Array<{
          id: string
          type?: string
          attributes?: {
            name?: string
            artistName?: string
            durationInMillis?: number
          }
        }>
      }
    }
  }>
  errors?: Array<{
    title?: string
    detail?: string
    status?: string
    code?: string
  }>
}

interface RawTracksPageResponse {
  next?: string
  data?: Array<{
    id: string
    type?: string
    attributes?: {
      name?: string
      artistName?: string
      durationInMillis?: number
    }
  }>
}

async function fetchPlaylistInternal(
  playlistId: string,
  storefront: string,
): Promise<PlaylistData> {
  using _ = infoSpan('apple_playlist', {
    playlist_id: playlistId,
    storefront,
  }).enter()

  const token = await getAppleMusicDeveloperToken()
  const sf = storefront.toLowerCase()
  const initialUrl = `https://amp-api.music.apple.com/v1/catalog/${encodeURIComponent(sf)}/playlists/${encodeURIComponent(playlistId)}`

  const headers = {
    Authorization: `Bearer ${token}`,
    Origin: 'https://music.apple.com',
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
  }

  const start = Date.now()
  let resp: Response
  try {
    resp = await fetch(initialUrl, {
      headers,
      signal: AbortSignal.timeout(20_000),
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - start
    error('Apple Music playlist request timed out or network failed', {
      playlist_id: playlistId,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    throw new Error(
      `Playlist lookup timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
    )
  }

  if (resp.status === 404) {
    throw new Error(`Playlist ${playlistId} not found on storefront '${sf}'`)
  }

  if (!resp.ok) {
    throw new Error(`Apple Music API returned HTTP ${resp.status}`)
  }

  const json = (await resp.json()) as RawPlaylistResponse
  const playlistItem = json.data?.[0]
  if (!playlistItem) {
    throw new Error(`No playlist found matching ID ${playlistId}`)
  }

  const title = playlistItem.attributes?.name || 'Untitled Playlist'
  const curatorName = playlistItem.attributes?.curatorName
  const description = playlistItem.attributes?.description?.standard

  const tracks: PlaylistTrack[] = []

  const initialTracks = playlistItem.relationships?.tracks?.data || []
  for (const t of initialTracks) {
    if (t.id) {
      tracks.push({
        id: t.id,
        title: t.attributes?.name || `Track ${t.id}`,
        artist: t.attributes?.artistName || 'Unknown Artist',
        duration: t.attributes?.durationInMillis
          ? Math.round(t.attributes.durationInMillis / 1000)
          : undefined,
      })
    }
  }

  // Handle pagination if tracks exceed page size
  let nextUrl = playlistItem.relationships?.tracks?.next
  while (nextUrl) {
    const fullNextUrl = nextUrl.startsWith('http')
      ? nextUrl
      : `https://amp-api.music.apple.com${nextUrl}`
    try {
      const pageResp = await fetch(fullNextUrl, {
        headers,
        signal: AbortSignal.timeout(15_000),
      })
      if (!pageResp.ok) break
      const pageJson = (await pageResp.json()) as RawTracksPageResponse
      const pageItems = pageJson.data || []
      for (const t of pageItems) {
        if (t.id) {
          tracks.push({
            id: t.id,
            title: t.attributes?.name || `Track ${t.id}`,
            artist: t.attributes?.artistName || 'Unknown Artist',
            duration: t.attributes?.durationInMillis
              ? Math.round(t.attributes.durationInMillis / 1000)
              : undefined,
          })
        }
      }
      nextUrl = pageJson.next
    } catch {
      break
    }
  }

  info('Apple Music playlist resolved', {
    playlist_id: playlistId,
    title,
    curator: curatorName,
    trackCount: tracks.length,
    elapsed_ms: Date.now() - start,
  })

  return {
    id: playlistId,
    title,
    curatorName,
    description,
    tracks,
  }
}

/**
 * Fetches Apple Music playlist metadata and all tracks, falling back to 'us' storefront
 * if regional lookup fails.
 */
export async function fetchPlaylistTracks(
  playlistId: string,
  storefront = 'us',
): Promise<PlaylistData> {
  const sf = (storefront || 'us').toLowerCase()
  try {
    return await fetchPlaylistInternal(playlistId, sf)
  } catch (err) {
    if (sf !== 'us') {
      debug('Retrying playlist lookup on US storefront fallback', {
        playlist_id: playlistId,
        original_storefront: sf,
        error: err instanceof Error ? err.message : String(err),
      })
      return await fetchPlaylistInternal(playlistId, 'us')
    }
    throw err
  }
}
