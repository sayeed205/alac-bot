import { debug, error, info, infoSpan } from '@/utils/logger.ts'

import type { AppleTrackMetadata } from './types.ts'

interface ItunesResult {
  wrapperType?: string
  kind?: string
  trackId?: number
  trackName?: string
  artistName?: string
  collectionId?: number
  collectionName?: string
  collectionArtistName?: string
  primaryGenreName?: string
  releaseDate?: string
  composerName?: string
  trackNumber?: number
  trackCount?: number
  discNumber?: number
  discCount?: number
  trackTimeMillis?: number
  trackExplicitness?: string
  collectionExplicitness?: string
  artworkUrl100?: string
}

function formatArtworkUrl(url?: string): string {
  if (!url) return ''
  return url.replace(/\d+x\d+bb/, '3000x3000bb')
}

async function doFetchTrackMeta(
  trackId: string,
  storefront: string,
): Promise<AppleTrackMetadata> {
  using _ = infoSpan('itunes_track', { track_id: trackId, storefront }).enter()

  const url = `https://itunes.apple.com/lookup?id=${encodeURIComponent(trackId)}&country=${encodeURIComponent(storefront)}`
  debug('Querying iTunes API for track...', { url })

  const start = Date.now()
  let resp: Response
  try {
    resp = await fetch(url, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: AbortSignal.timeout(20_000),
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - start
    error('iTunes track lookup timed out / network error', {
      track_id: trackId,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    throw new Error(
      `iTunes track lookup timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
    )
  }

  if (!resp.ok) {
    error('iTunes lookup HTTP failure', {
      track_id: trackId,
      status: resp.status,
    })
    throw new Error(`iTunes lookup failed (HTTP ${resp.status})`)
  }

  const data = (await resp.json()) as { results?: ItunesResult[] }
  const item = (data.results || []).find(
    (r) => r.wrapperType === 'track' || r.kind === 'song',
  )

  if (!item) {
    error('No matching track found in iTunes catalog', { track_id: trackId })
    throw new Error(`iTunes found no song matching track ID ${trackId}`)
  }

  const meta = mapItunesItem(item)
  info('iTunes metadata resolved', {
    track_id: trackId,
    artist: meta.artist,
    title: meta.title,
    duration: meta.duration,
    elapsed_ms: Date.now() - start,
  })

  return meta
}

export async function fetchTrackMeta(
  trackId: string,
  storefront = 'us',
): Promise<AppleTrackMetadata> {
  const sf = (storefront || 'us').toLowerCase()
  try {
    return await doFetchTrackMeta(trackId, sf)
  } catch (err) {
    if (sf !== 'us') {
      debug('Retrying track lookup on US storefront fallback', {
        track_id: trackId,
        original_storefront: sf,
        error: err instanceof Error ? err.message : String(err),
      })
      return await doFetchTrackMeta(trackId, 'us')
    }
    throw err
  }
}

async function doFetchAlbumTracks(
  collectionId: string,
  storefront: string,
): Promise<{ album: AppleTrackMetadata; tracks: AppleTrackMetadata[] }> {
  using _ = infoSpan('itunes_album', {
    collection_id: collectionId,
    storefront,
  }).enter()

  const url = `https://itunes.apple.com/lookup?id=${encodeURIComponent(collectionId)}&entity=song&country=${encodeURIComponent(storefront)}`
  debug('Querying iTunes API for album collection...', { url })

  const start = Date.now()
  let resp: Response
  try {
    resp = await fetch(url, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: AbortSignal.timeout(25_000),
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - start
    error('iTunes album lookup timed out / network error', {
      collection_id: collectionId,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    throw new Error(
      `iTunes album lookup timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
    )
  }

  if (!resp.ok) {
    error('iTunes album lookup HTTP failure', {
      collection_id: collectionId,
      status: resp.status,
    })
    throw new Error(`iTunes album lookup failed (HTTP ${resp.status})`)
  }

  const data = (await resp.json()) as { results?: ItunesResult[] }
  const results = data.results || []

  const collectionItem = results.find((r) => r.wrapperType === 'collection')
  const trackItems = results.filter(
    (r) => r.wrapperType === 'track' || r.kind === 'song',
  )

  if (trackItems.length === 0) {
    error('No tracks found for collection', { collection_id: collectionId })
    throw new Error(`iTunes found no tracks for collection ${collectionId}`)
  }

  const tracks = trackItems.map(mapItunesItem)
  const firstTrack = tracks[0]
  if (!firstTrack) {
    throw new Error(`Failed to parse tracks for collection ${collectionId}`)
  }

  const albumMeta: AppleTrackMetadata = collectionItem
    ? {
        id: String(collectionItem.collectionId || collectionId),
        title: collectionItem.collectionName || '',
        artist: collectionItem.artistName || '',
        album: collectionItem.collectionName || '',
        albumArtist: collectionItem.artistName || '',
        genre: collectionItem.primaryGenreName,
        releaseDate: (collectionItem.releaseDate || '').slice(0, 10),
        duration: 0,
        explicit: collectionItem.collectionExplicitness === 'explicit',
        artworkUrl: formatArtworkUrl(collectionItem.artworkUrl100),
      }
    : firstTrack

  info('iTunes album collection resolved', {
    collection_id: collectionId,
    album: albumMeta.album,
    artist: albumMeta.artist,
    track_count: tracks.length,
    elapsed_ms: Date.now() - start,
  })

  return { album: albumMeta, tracks }
}

export async function fetchAlbumTracks(
  collectionId: string,
  storefront = 'us',
): Promise<{ album: AppleTrackMetadata; tracks: AppleTrackMetadata[] }> {
  const sf = (storefront || 'us').toLowerCase()
  try {
    return await doFetchAlbumTracks(collectionId, sf)
  } catch (err) {
    if (sf !== 'us') {
      debug('Retrying album lookup on US storefront fallback', {
        collection_id: collectionId,
        original_storefront: sf,
        error: err instanceof Error ? err.message : String(err),
      })
      return await doFetchAlbumTracks(collectionId, 'us')
    }
    throw err
  }
}

async function doFetchArtistTracks(
  artistId: string,
  storefront: string,
): Promise<{
  artistId: string
  artistName: string
  tracks: AppleTrackMetadata[]
}> {
  using _ = infoSpan('itunes_artist', {
    artist_id: artistId,
    storefront,
  }).enter()

  const url = `https://itunes.apple.com/lookup?id=${encodeURIComponent(artistId)}&entity=album&limit=200&country=${encodeURIComponent(storefront)}`
  debug('Querying iTunes API for artist discography...', { url })

  const start = Date.now()
  let resp: Response
  try {
    resp = await fetch(url, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: AbortSignal.timeout(30_000),
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - start
    error('iTunes artist lookup timed out / network error', {
      artist_id: artistId,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    throw new Error(
      `iTunes artist lookup timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
    )
  }

  if (!resp.ok) {
    error('iTunes artist lookup HTTP failure', {
      artist_id: artistId,
      status: resp.status,
    })
    throw new Error(`iTunes artist lookup failed (HTTP ${resp.status})`)
  }

  const data = (await resp.json()) as { results?: ItunesResult[] }
  const results = data.results || []

  const artistItem = results.find((r) => r.wrapperType === 'artist')
  const collectionItems = results.filter((r) => r.wrapperType === 'collection')

  let artistName = artistItem?.artistName || ''

  const allTracks: AppleTrackMetadata[] = []
  const seenTrackIds = new Set<string>()

  if (collectionItems.length > 0) {
    const collectionIds = collectionItems
      .map((c) => c.collectionId)
      .filter((id): id is number => typeof id === 'number')

    const chunkSize = 25
    for (let i = 0; i < collectionIds.length; i += chunkSize) {
      const chunk = collectionIds.slice(i, i + chunkSize)
      const batchUrl = `https://itunes.apple.com/lookup?id=${chunk.join(',')}&entity=song&country=${encodeURIComponent(storefront)}`

      try {
        const batchResp = await fetch(batchUrl, {
          headers: {
            'User-Agent':
              'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
          },
          signal: AbortSignal.timeout(20_000),
        })

        if (batchResp.ok) {
          const batchData = (await batchResp.json()) as {
            results?: ItunesResult[]
          }
          const batchResults = batchData.results || []

          for (const item of batchResults) {
            if (item.wrapperType === 'track' || item.kind === 'song') {
              const trackId = String(item.trackId || '')
              if (trackId && !seenTrackIds.has(trackId)) {
                seenTrackIds.add(trackId)
                allTracks.push(mapItunesItem(item))
              }
            }
          }
        }
      } catch (batchErr: unknown) {
        debug('Error fetching artist collection batch, continuing...', {
          error: String(batchErr),
        })
      }
    }
  }

  // If no tracks were resolved via collections, fall back to entity=song directly on artist
  if (allTracks.length === 0) {
    const songsUrl = `https://itunes.apple.com/lookup?id=${encodeURIComponent(artistId)}&entity=song&limit=200&country=${encodeURIComponent(storefront)}`
    try {
      const songResp = await fetch(songsUrl, {
        headers: {
          'User-Agent':
            'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
        },
        signal: AbortSignal.timeout(20_000),
      })
      if (songResp.ok) {
        const songData = (await songResp.json()) as {
          results?: ItunesResult[]
        }
        const songResults = songData.results || []
        if (!artistName) {
          artistName =
            songResults.find((r) => r.wrapperType === 'artist')?.artistName ||
            ''
        }
        for (const item of songResults) {
          if (item.wrapperType === 'track' || item.kind === 'song') {
            const trackId = String(item.trackId || '')
            if (trackId && !seenTrackIds.has(trackId)) {
              seenTrackIds.add(trackId)
              allTracks.push(mapItunesItem(item))
            }
          }
        }
      }
    } catch {}
  }

  if (allTracks.length === 0) {
    error('No tracks found for artist', { artist_id: artistId })
    throw new Error(`iTunes found no tracks for artist ${artistId}`)
  }

  if (!artistName && allTracks[0]) {
    artistName = allTracks[0].artist
  }

  info('iTunes artist discography resolved', {
    artist_id: artistId,
    artist: artistName,
    track_count: allTracks.length,
    elapsed_ms: Date.now() - start,
  })

  return {
    artistId,
    artistName: artistName || 'Unknown Artist',
    tracks: allTracks,
  }
}

export async function fetchArtistTracks(
  artistId: string,
  storefront = 'us',
): Promise<{
  artistId: string
  artistName: string
  tracks: AppleTrackMetadata[]
}> {
  const sf = (storefront || 'us').toLowerCase()
  try {
    return await doFetchArtistTracks(artistId, sf)
  } catch (err) {
    if (sf !== 'us') {
      debug('Retrying artist lookup on US storefront fallback', {
        artist_id: artistId,
        original_storefront: sf,
        error: err instanceof Error ? err.message : String(err),
      })
      return await doFetchArtistTracks(artistId, 'us')
    }
    throw err
  }
}

async function doSearchItunesCatalog(
  term: string,
  limit: number,
  storefront: string,
): Promise<AppleTrackMetadata[]> {
  using _ = infoSpan('itunes_search', { term, limit, storefront }).enter()

  const url = `https://itunes.apple.com/search?term=${encodeURIComponent(term)}&entity=song&limit=${encodeURIComponent(limit)}&country=${encodeURIComponent(storefront)}`
  debug('Querying iTunes search API...', { url })

  const start = Date.now()
  let resp: Response
  try {
    resp = await fetch(url, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: AbortSignal.timeout(15_000),
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - start
    error('iTunes search timed out / network error', {
      term,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    return []
  }

  if (!resp.ok) {
    error('iTunes search HTTP failure', { term, status: resp.status })
    return []
  }

  const data = (await resp.json()) as { results?: ItunesResult[] }
  const tracks = (data.results || [])
    .filter((r) => r.wrapperType === 'track' || r.kind === 'song')
    .map(mapItunesItem)

  info('iTunes search resolved', {
    term,
    matches: tracks.length,
    elapsed_ms: Date.now() - start,
  })

  return tracks
}

export async function searchItunesCatalog(
  term: string,
  limit = 5,
  storefront = 'us',
): Promise<AppleTrackMetadata[]> {
  const sf = (storefront || 'us').toLowerCase()
  const results = await doSearchItunesCatalog(term, limit, sf)
  if (results.length === 0 && sf !== 'us') {
    debug('Retrying catalog search on US storefront fallback', {
      term,
      original_storefront: sf,
    })
    return await doSearchItunesCatalog(term, limit, 'us')
  }
  return results
}

function mapItunesItem(item: ItunesResult): AppleTrackMetadata {
  const artwork = formatArtworkUrl(item.artworkUrl100)

  return {
    id: String(item.trackId || ''),
    title: item.trackName || '',
    artist: item.artistName || '',
    album: item.collectionName || '',
    albumArtist: item.collectionArtistName || item.artistName || '',
    genre: item.primaryGenreName,
    releaseDate: (item.releaseDate || '').slice(0, 10),
    composer: item.composerName,
    trackNumber: item.trackNumber,
    trackCount: item.trackCount,
    discNumber: item.discNumber,
    discCount: item.discCount,
    duration: Math.round((item.trackTimeMillis || 0) / 1000),
    explicit: item.trackExplicitness === 'explicit',
    artworkUrl: artwork,
  }
}
