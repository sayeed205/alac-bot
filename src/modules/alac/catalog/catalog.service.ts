import type { AppleTrackMetadata } from '@/modules/alac/types.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'

const REGIONAL_STOREFRONTS = ['jp', 'gb', 'in', 'ca', 'de', 'fr', 'au'] as const

interface ItunesRawResult {
  wrapperType?: string
  kind?: string
  trackId?: number
  collectionId?: number
  artistId?: number
  trackName?: string
  collectionName?: string
  artistName?: string
  primaryGenreName?: string
  releaseDate?: string
  trackNumber?: number
  trackCount?: number
  discNumber?: number
  discCount?: number
  trackTimeMillis?: number
  trackExplicitness?: string
  collectionExplicitness?: string
  artworkUrl100?: string
}

export interface ChartAlbumItem {
  id: string
  title: string
  artist: string
  url: string
  artworkUrl?: string
  releaseDate?: string
  genre?: string
}

export interface ICatalogService {
  fetchTrackMeta(
    trackId: string,
    storefront?: string,
  ): Promise<AppleTrackMetadata>
  fetchAlbumTracks(
    collectionId: string,
    storefront?: string,
  ): Promise<{ album: AppleTrackMetadata; tracks: AppleTrackMetadata[] }>
  fetchArtistTracks(
    artistId: string,
    storefront?: string,
  ): Promise<{
    artistId: string
    artistName: string
    tracks: AppleTrackMetadata[]
  }>
  searchCatalog(
    term: string,
    limit?: number,
    storefront?: string,
  ): Promise<AppleTrackMetadata[]>
  fetchChartsAlbums(
    storefront?: string,
    limit?: number,
  ): Promise<ChartAlbumItem[]>
  clearCache(): void
}

interface CacheEntry<T> {
  value: T
  expiresAt: number
}

export class CatalogService implements ICatalogService {
  private readonly cache = new Map<string, CacheEntry<unknown>>()
  private readonly maxCacheSize: number
  private readonly defaultTtlMs: number

  constructor(maxCacheSize = 500, defaultTtlMs = 10 * 60 * 1000) {
    this.maxCacheSize = maxCacheSize
    this.defaultTtlMs = defaultTtlMs
  }

  private getFromCache<T>(key: string): T | null {
    const entry = this.cache.get(key)
    if (!entry) return null
    if (Date.now() > entry.expiresAt) {
      this.cache.delete(key)
      return null
    }
    // Refresh LRU order
    this.cache.delete(key)
    this.cache.set(key, entry)
    return entry.value as T
  }

  private setInCache<T>(
    key: string,
    value: T,
    ttlMs = this.defaultTtlMs,
  ): void {
    if (this.cache.size >= this.maxCacheSize) {
      const oldestKey = this.cache.keys().next().value
      if (oldestKey) this.cache.delete(oldestKey)
    }
    this.cache.set(key, {
      value,
      expiresAt: Date.now() + ttlMs,
    })
  }

  clearCache(): void {
    this.cache.clear()
  }

  private formatArtworkUrl(url?: string): string {
    if (!url) return ''
    return url.replace(/\d+x\d+bb/, '3000x3000bb')
  }

  private mapItunesItem(item: ItunesRawResult): AppleTrackMetadata {
    return {
      id: String(item.trackId || ''),
      title: item.trackName || '',
      artist: item.artistName || '',
      album: item.collectionName || '',
      albumArtist: item.artistName || '',
      genre: item.primaryGenreName,
      releaseDate: (item.releaseDate || '').slice(0, 10),
      duration: Math.round((item.trackTimeMillis || 0) / 1000),
      trackNumber: item.trackNumber,
      trackCount: item.trackCount,
      discNumber: item.discNumber,
      discCount: item.discCount,
      explicit: item.trackExplicitness === 'explicit',
      artworkUrl: this.formatArtworkUrl(item.artworkUrl100),
    }
  }

  private async doFetchTrackMeta(
    trackId: string,
    storefront: string,
  ): Promise<AppleTrackMetadata> {
    using _ = infoSpan('itunes_track', {
      track_id: trackId,
      storefront,
    }).enter()

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
        signal: AbortSignal.timeout(15_000),
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

    const data = (await resp.json()) as { results?: ItunesRawResult[] }
    const results = data.results || []
    const trackItem = results.find(
      (r) =>
        r.wrapperType === 'track' ||
        r.kind === 'song' ||
        String(r.trackId) === String(trackId),
    )

    if (!trackItem) {
      error('Track not found in iTunes response', { track_id: trackId })
      throw new Error(`iTunes found no song matching track ID ${trackId}`)
    }

    const meta = this.mapItunesItem(trackItem)
    meta.id = String(trackItem.trackId || trackId)

    info('iTunes metadata resolved', {
      track_id: trackId,
      artist: meta.artist,
      title: meta.title,
      duration: meta.duration,
      elapsed_ms: Date.now() - start,
    })

    return meta
  }

  async fetchTrackMeta(
    trackId: string,
    storefront = 'us',
  ): Promise<AppleTrackMetadata> {
    const sf = (storefront || 'us').toLowerCase()
    const cacheKey = `track:${sf}:${trackId}`
    const cached = this.getFromCache<AppleTrackMetadata>(cacheKey)
    if (cached) return cached

    try {
      const meta = await this.doFetchTrackMeta(trackId, sf)
      this.setInCache(cacheKey, meta)
      return meta
    } catch (err) {
      if (sf !== 'us') {
        debug('Retrying track lookup on US storefront fallback', {
          track_id: trackId,
          original_storefront: sf,
          error: err instanceof Error ? err.message : String(err),
        })
        try {
          const fallbackMeta = await this.doFetchTrackMeta(trackId, 'us')
          this.setInCache(cacheKey, fallbackMeta)
          return fallbackMeta
        } catch {
          // Fall through to regional fallbacks
        }
      }

      const regionalFallbacks = REGIONAL_STOREFRONTS.filter(
        (s) => (s as string) !== sf,
      )
      for (const fallbackSf of regionalFallbacks) {
        try {
          debug('Retrying track lookup on regional storefront fallback', {
            track_id: trackId,
            fallback_storefront: fallbackSf,
          })
          const fallbackMeta = await this.doFetchTrackMeta(trackId, fallbackSf)
          this.setInCache(cacheKey, fallbackMeta)
          return fallbackMeta
        } catch {
          // Try next storefront
        }
      }

      throw err
    }
  }

  private async doFetchAlbumTracks(
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

    const data = (await resp.json()) as { results?: ItunesRawResult[] }
    const results = data.results || []

    const collectionItem = results.find((r) => r.wrapperType === 'collection')
    const trackItems = results.filter(
      (r) => r.wrapperType === 'track' || r.kind === 'song',
    )

    if (trackItems.length === 0) {
      error('No tracks found for collection', { collection_id: collectionId })
      throw new Error(`iTunes found no tracks for collection ${collectionId}`)
    }

    const tracks = trackItems.map((item) => this.mapItunesItem(item))
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
          artworkUrl: this.formatArtworkUrl(collectionItem.artworkUrl100),
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

  async fetchAlbumTracks(
    collectionId: string,
    storefront = 'us',
  ): Promise<{ album: AppleTrackMetadata; tracks: AppleTrackMetadata[] }> {
    const sf = (storefront || 'us').toLowerCase()
    const cacheKey = `album:${sf}:${collectionId}`
    const cached = this.getFromCache<{
      album: AppleTrackMetadata
      tracks: AppleTrackMetadata[]
    }>(cacheKey)
    if (cached) return cached

    try {
      const result = await this.doFetchAlbumTracks(collectionId, sf)
      this.setInCache(cacheKey, result)
      return result
    } catch (err) {
      if (sf !== 'us') {
        try {
          debug('Retrying album lookup on US storefront fallback', {
            collection_id: collectionId,
            original_storefront: sf,
            error: err instanceof Error ? err.message : String(err),
          })
          const result = await this.doFetchAlbumTracks(collectionId, 'us')
          this.setInCache(cacheKey, result)
          return result
        } catch {
          // Fall through to regional fallbacks
        }
      }

      const fallbacks = REGIONAL_STOREFRONTS.filter((s) => (s as string) !== sf)
      for (const fallbackSf of fallbacks) {
        try {
          debug('Retrying album lookup on regional storefront fallback', {
            collection_id: collectionId,
            fallback_storefront: fallbackSf,
          })
          const result = await this.doFetchAlbumTracks(collectionId, fallbackSf)
          this.setInCache(cacheKey, result)
          return result
        } catch {
          // Try next storefront
        }
      }

      throw err
    }
  }

  private async doFetchArtistTracks(
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

    const data = (await resp.json()) as { results?: ItunesRawResult[] }
    const results = data.results || []

    const artistItem = results.find((r) => r.wrapperType === 'artist')
    const collectionItems = results.filter(
      (r) => r.wrapperType === 'collection',
    )

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
              results?: ItunesRawResult[]
            }
            const batchResults = batchData.results || []

            for (const item of batchResults) {
              if (item.wrapperType === 'track' || item.kind === 'song') {
                const trackId = String(item.trackId || '')
                if (trackId && !seenTrackIds.has(trackId)) {
                  seenTrackIds.add(trackId)
                  allTracks.push(this.mapItunesItem(item))
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
            results?: ItunesRawResult[]
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
                allTracks.push(this.mapItunesItem(item))
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

  async fetchArtistTracks(
    artistId: string,
    storefront = 'us',
  ): Promise<{
    artistId: string
    artistName: string
    tracks: AppleTrackMetadata[]
  }> {
    const sf = (storefront || 'us').toLowerCase()
    const cacheKey = `artist:${sf}:${artistId}`
    const cached = this.getFromCache<{
      artistId: string
      artistName: string
      tracks: AppleTrackMetadata[]
    }>(cacheKey)
    if (cached) return cached

    try {
      const result = await this.doFetchArtistTracks(artistId, sf)
      this.setInCache(cacheKey, result)
      return result
    } catch (err) {
      if (sf !== 'us') {
        try {
          debug('Retrying artist lookup on US storefront fallback', {
            artist_id: artistId,
            original_storefront: sf,
            error: err instanceof Error ? err.message : String(err),
          })
          const result = await this.doFetchArtistTracks(artistId, 'us')
          this.setInCache(cacheKey, result)
          return result
        } catch {
          // Fall through to regional fallbacks
        }
      }

      const fallbacks = REGIONAL_STOREFRONTS.filter((s) => (s as string) !== sf)
      for (const fallbackSf of fallbacks) {
        try {
          debug('Retrying artist lookup on regional storefront fallback', {
            artist_id: artistId,
            fallback_storefront: fallbackSf,
          })
          const result = await this.doFetchArtistTracks(artistId, fallbackSf)
          this.setInCache(cacheKey, result)
          return result
        } catch {
          // Try next storefront
        }
      }

      throw err
    }
  }

  private async doSearchItunesCatalog(
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

    const data = (await resp.json()) as { results?: ItunesRawResult[] }
    const tracks = (data.results || [])
      .filter((r) => r.wrapperType === 'track' || r.kind === 'song')
      .map((item) => this.mapItunesItem(item))

    info('iTunes search resolved', {
      term,
      matches: tracks.length,
      elapsed_ms: Date.now() - start,
    })

    return tracks
  }

  async searchCatalog(
    term: string,
    limit = 5,
    storefront = 'us',
  ): Promise<AppleTrackMetadata[]> {
    const sf = (storefront || 'us').toLowerCase()
    const cleanTerm = term.trim().toLowerCase()
    const cacheKey = `search:${sf}:${limit}:${cleanTerm}`
    const cached = this.getFromCache<AppleTrackMetadata[]>(cacheKey)
    if (cached) return cached

    let results = await this.doSearchItunesCatalog(term, limit, sf)
    if (results.length === 0 && sf !== 'us') {
      debug('Retrying catalog search on US storefront fallback', {
        term,
        original_storefront: sf,
      })
      results = await this.doSearchItunesCatalog(term, limit, 'us')
    }

    if (results.length === 0) {
      const fallbacks = REGIONAL_STOREFRONTS.filter((s) => (s as string) !== sf)
      for (const fallbackSf of fallbacks) {
        results = await this.doSearchItunesCatalog(term, limit, fallbackSf)
        if (results.length > 0) break
      }
    }

    this.setInCache(cacheKey, results)
    return results
  }

  async fetchChartsAlbums(
    storefront = 'us',
    limit = 50,
  ): Promise<ChartAlbumItem[]> {
    const sf = (storefront || 'us').toLowerCase()
    const cacheKey = `charts:${sf}:${limit}`
    const cached = this.getFromCache<ChartAlbumItem[]>(cacheKey)
    if (cached) return cached

    const url = `https://rss.marketingtools.apple.com/api/v2/${encodeURIComponent(sf)}/music/most-played/${encodeURIComponent(limit)}/albums.json`
    const res = await fetch(url, {
      headers: {
        'User-Agent': 'Mozilla/5.0',
      },
      signal: AbortSignal.timeout(15_000),
    })

    if (!res.ok) {
      throw new Error(`Failed to fetch Apple Music charts (HTTP ${res.status})`)
    }

    const data = (await res.json()) as {
      feed?: {
        results?: Array<{
          id: string
          name: string
          artistName: string
          url: string
          artworkUrl100?: string
          releaseDate?: string
          genres?: Array<{ name: string }>
        }>
      }
    }

    const results = data.feed?.results ?? []
    const mapped: ChartAlbumItem[] = results.map((r) => ({
      id: r.id,
      title: r.name,
      artist: r.artistName,
      url: r.url,
      artworkUrl: r.artworkUrl100
        ? this.formatArtworkUrl(r.artworkUrl100)
        : undefined,
      releaseDate: r.releaseDate,
      genre: r.genres?.[0]?.name,
    }))

    this.setInCache(cacheKey, mapped, this.defaultTtlMs)
    return mapped
  }
}

export const catalogService = new CatalogService()
