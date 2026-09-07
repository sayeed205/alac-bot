import { afterEach, beforeEach, describe, expect, it } from 'bun:test'

import { CatalogService, catalogService } from '@/modules/alac/catalog/index.ts'

describe('CatalogService', () => {
  const originalFetch = globalThis.fetch

  const setFetch = (fn: unknown) => {
    globalThis.fetch = fn as typeof fetch
  }

  beforeEach(() => {
    catalogService.clearCache()
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  it('resolves track metadata and caches results', async () => {
    let fetchCount = 0
    setFetch(async (url: string | URL | Request) => {
      fetchCount++
      const urlStr = String(url)
      if (urlStr.includes('lookup') && urlStr.includes('id=12345')) {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                trackId: 12345,
                trackName: 'Test Song',
                artistName: 'Test Artist',
                collectionName: 'Test Album',
                trackTimeMillis: 210000,
                trackNumber: 1,
                trackCount: 10,
                artworkUrl100: 'https://example.com/100x100bb.jpg',
              },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response(JSON.stringify({ results: [] }), { status: 200 })
    })

    const svc = new CatalogService(10, 60000)
    const track1 = await svc.fetchTrackMeta('12345', 'us')
    expect(track1.id).toBe('12345')
    expect(track1.title).toBe('Test Song')
    expect(track1.duration).toBe(210)
    expect(track1.artworkUrl).toBe('https://example.com/3000x3000bb.jpg')
    expect(fetchCount).toBe(1)

    // Second call should hit in-memory cache without calling fetch again
    const track2 = await svc.fetchTrackMeta('12345', 'us')
    expect(track2.title).toBe('Test Song')
    expect(fetchCount).toBe(1)
  })

  it('falls back to US and regional storefronts on lookup error', async () => {
    const attemptedStorefronts: string[] = []
    setFetch(async (url: string | URL | Request) => {
      const urlStr = String(url)
      const match = urlStr.match(/country=([a-z]+)/)
      const sf = match ? match[1] : 'unknown'
      attemptedStorefronts.push(sf)

      if (sf === 'jp') {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                trackId: 99999,
                trackName: 'JP Track',
                artistName: 'JP Artist',
              },
            ],
          }),
          { status: 200 },
        )
      }

      return new Response(JSON.stringify({ results: [] }), { status: 404 })
    })

    const svc = new CatalogService()
    const track = await svc.fetchTrackMeta('99999', 'de')
    expect(track.title).toBe('JP Track')
    expect(attemptedStorefronts).toContain('de')
    expect(attemptedStorefronts).toContain('us')
    expect(attemptedStorefronts).toContain('jp')
  })

  it('falls back to regional storefronts for album tracks', async () => {
    const attemptedStorefronts: string[] = []
    setFetch(async (url: string | URL | Request) => {
      const urlStr = String(url)
      const match = urlStr.match(/country=([a-z]+)/)
      const sf = match ? match[1] : 'unknown'
      attemptedStorefronts.push(sf)

      if (sf === 'gb') {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 8888,
                collectionName: 'GB Album',
                artistName: 'GB Artist',
              },
              {
                wrapperType: 'track',
                trackId: 88881,
                trackName: 'GB Track 1',
                artistName: 'GB Artist',
              },
            ],
          }),
          { status: 200 },
        )
      }

      return new Response(JSON.stringify({ results: [] }), { status: 404 })
    })

    const svc = new CatalogService()
    const res = await svc.fetchAlbumTracks('8888', 'fr')
    expect(res.album.album).toBe('GB Album')
    expect(res.tracks.length).toBe(1)
    expect(attemptedStorefronts).toContain('fr')
    expect(attemptedStorefronts).toContain('us')
    expect(attemptedStorefronts).toContain('gb')
  })

  it('searches catalog and caches search results', async () => {
    let fetchCount = 0
    setFetch(async (url: string | URL | Request) => {
      fetchCount++
      const urlStr = String(url)
      if (urlStr.includes('search')) {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                trackId: 101,
                trackName: 'Radioactive',
                artistName: 'Imagine Dragons',
              },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response(JSON.stringify({ results: [] }), { status: 200 })
    })

    const svc = new CatalogService()
    const res1 = await svc.searchCatalog('Radioactive', 5, 'us')
    expect(res1.length).toBe(1)
    expect(res1[0]?.title).toBe('Radioactive')
    expect(fetchCount).toBe(1)

    // Second call hits cache
    const res2 = await svc.searchCatalog('Radioactive', 5, 'us')
    expect(res2.length).toBe(1)
    expect(fetchCount).toBe(1)
  })

  it('fetches and caches charts albums', async () => {
    let fetchCount = 0
    setFetch(async (url: string | URL | Request) => {
      fetchCount++
      const urlStr = String(url)
      if (urlStr.includes('rss.marketingtools.apple.com')) {
        return new Response(
          JSON.stringify({
            feed: {
              results: [
                {
                  id: 'album_1',
                  name: 'Top Chart Album',
                  artistName: 'Top Chart Artist',
                  url: 'https://music.apple.com/us/album/top/album_1',
                  artworkUrl100: 'https://example.com/100x100bb.jpg',
                  releaseDate: '2026-01-01',
                  genres: [{ name: 'Pop' }],
                },
              ],
            },
          }),
          { status: 200 },
        )
      }
      return new Response(JSON.stringify({}), { status: 404 })
    })

    const svc = new CatalogService()
    const charts1 = await svc.fetchChartsAlbums('us', 10)
    expect(charts1.length).toBe(1)
    expect(charts1[0]?.id).toBe('album_1')
    expect(charts1[0]?.title).toBe('Top Chart Album')
    expect(charts1[0]?.artworkUrl).toBe('https://example.com/3000x3000bb.jpg')
    expect(fetchCount).toBe(1)

    // Cache hit
    const charts2 = await svc.fetchChartsAlbums('us', 10)
    expect(charts2.length).toBe(1)
    expect(fetchCount).toBe(1)
  })

  it('evicts oldest items when maxCacheSize is reached (LRU)', async () => {
    setFetch(async (url: string | URL | Request) => {
      const urlStr = String(url)
      const match = urlStr.match(/id=([0-9]+)/)
      const id = match ? match[1] : '0'
      return new Response(
        JSON.stringify({
          results: [
            {
              wrapperType: 'track',
              trackId: Number(id),
              trackName: `Song ${id}`,
              artistName: 'Artist',
            },
          ],
        }),
        { status: 200 },
      )
    })

    const svc = new CatalogService(2, 60000) // max 2 items
    await svc.fetchTrackMeta('1', 'us')
    await svc.fetchTrackMeta('2', 'us')

    // Access 1 to make 2 the oldest
    await svc.fetchTrackMeta('1', 'us')

    // Add 3, which should evict 2
    await svc.fetchTrackMeta('3', 'us')

    let fetchCount = 0
    const currentFetch = globalThis.fetch
    setFetch(async (url: string | URL | Request) => {
      fetchCount++
      return currentFetch(url)
    })

    // 1 should still be in cache (0 extra fetches)
    await svc.fetchTrackMeta('1', 'us')
    expect(fetchCount).toBe(0)

    // 2 was evicted, so fetching 2 triggers a new fetch
    await svc.fetchTrackMeta('2', 'us')
    expect(fetchCount).toBe(1)
  })
})
