import { afterEach, describe, expect, it } from 'bun:test'

import {
  fetchAlbumTracks,
  fetchArtistTracks,
  fetchTrackMeta,
  searchItunesCatalog,
} from '@/modules/alac/itunes.ts'

describe('iTunes API Service', () => {
  const originalFetch = globalThis.fetch

  const setFetch = (fn: unknown) => {
    globalThis.fetch = fn as typeof fetch
  }

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  describe('fetchTrackMeta', () => {
    it('fetches and maps track metadata successfully', async () => {
      setFetch(async (input: RequestInfo | URL) => {
        const urlStr = String(input)
        expect(urlStr).toContain('id=1440857781')
        expect(urlStr).toContain('country=us')

        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                kind: 'song',
                trackId: 1440857781,
                trackName: 'In My Blood',
                artistName: 'Shawn Mendes',
                collectionId: 1440857774,
                collectionName: 'Shawn Mendes',
                collectionArtistName: 'Shawn Mendes',
                primaryGenreName: 'Pop',
                releaseDate: '2018-03-22T07:00:00Z',
                composerName: 'Shawn Mendes & Teddy Geiger',
                trackNumber: 1,
                trackCount: 14,
                discNumber: 1,
                discCount: 1,
                trackTimeMillis: 211360,
                trackExplicitness: 'notExplicit',
                artworkUrl100:
                  'https://is1-ssl.mzstatic.com/image/thumb/Music125/v4/xx/100x100bb.jpg',
              },
            ],
          }),
          { status: 200 },
        )
      })

      const meta = await fetchTrackMeta('1440857781', 'us')
      expect(meta.id).toBe('1440857781')
      expect(meta.title).toBe('In My Blood')
      expect(meta.artist).toBe('Shawn Mendes')
      expect(meta.album).toBe('Shawn Mendes')
      expect(meta.genre).toBe('Pop')
      expect(meta.releaseDate).toBe('2018-03-22')
      expect(meta.duration).toBe(211)
      expect(meta.explicit).toBe(false)
      expect(meta.artworkUrl).toBe(
        'https://is1-ssl.mzstatic.com/image/thumb/Music125/v4/xx/3000x3000bb.jpg',
      )
    })

    it('falls back to US storefront when regional track lookup returns no results', async () => {
      let callCount = 0
      setFetch(async (input: RequestInfo | URL) => {
        callCount++
        const urlStr = String(input)
        if (urlStr.includes('country=jp')) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 })
        }
        if (urlStr.includes('country=us')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 12345,
                  trackName: 'US Song',
                  artistName: 'US Artist',
                  collectionName: 'US Album',
                },
              ],
            }),
            { status: 200 },
          )
        }
        return new Response(null, { status: 404 })
      })

      const meta = await fetchTrackMeta('12345', 'jp')
      expect(callCount).toBe(2)
      expect(meta.title).toBe('US Song')
    })

    it('throws error when no track is found in catalog', async () => {
      setFetch(
        async () =>
          new Response(JSON.stringify({ results: [] }), { status: 200 }),
      )

      expect(fetchTrackMeta('9999999999')).rejects.toThrow(
        'iTunes found no song matching track ID 9999999999',
      )
    })

    it('throws error on non-ok HTTP status', async () => {
      setFetch(async () => new Response('Internal Error', { status: 502 }))

      expect(fetchTrackMeta('12345')).rejects.toThrow(
        'iTunes lookup failed (HTTP 502)',
      )
    })

    it('throws error on network timeout or fetch abort', async () => {
      setFetch(async () => {
        throw new Error('Connection refused')
      })

      expect(fetchTrackMeta('12345')).rejects.toThrow(
        'iTunes track lookup timed out',
      )
    })
  })

  describe('fetchAlbumTracks', () => {
    it('fetches album collection with tracks successfully', async () => {
      setFetch(async (input: RequestInfo | URL) => {
        const urlStr = String(input)
        expect(urlStr).toContain('id=1440857774')
        expect(urlStr).toContain('entity=song')

        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 1440857774,
                collectionName: 'Shawn Mendes Deluxe',
                artistName: 'Shawn Mendes',
                primaryGenreName: 'Pop',
                releaseDate: '2018-03-22T07:00:00Z',
                collectionExplicitness: 'explicit',
                artworkUrl100: 'https://example.com/100x100bb.jpg',
              },
              {
                wrapperType: 'track',
                kind: 'song',
                trackId: 1440857781,
                trackName: 'Track 1',
                artistName: 'Shawn Mendes',
                collectionName: 'Shawn Mendes Deluxe',
                trackTimeMillis: 180000,
              },
              {
                wrapperType: 'track',
                kind: 'song',
                trackId: 1440857782,
                trackName: 'Track 2',
                artistName: 'Shawn Mendes',
                collectionName: 'Shawn Mendes Deluxe',
                trackTimeMillis: 200000,
              },
            ],
          }),
          { status: 200 },
        )
      })

      const result = await fetchAlbumTracks('1440857774', 'us')
      expect(result.album.id).toBe('1440857774')
      expect(result.album.title).toBe('Shawn Mendes Deluxe')
      expect(result.album.explicit).toBe(true)
      expect(result.album.artworkUrl).toBe(
        'https://example.com/3000x3000bb.jpg',
      )
      expect(result.tracks.length).toBe(2)
      expect(result.tracks[0]?.title).toBe('Track 1')
      expect(result.tracks[1]?.title).toBe('Track 2')
    })

    it('falls back to first track when collection item is missing', async () => {
      setFetch(async () => {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                kind: 'song',
                trackId: 1440857781,
                trackName: 'Track 1',
                artistName: 'Artist 1',
                collectionName: 'Solo EP',
                trackTimeMillis: 150000,
              },
            ],
          }),
          { status: 200 },
        )
      })

      const result = await fetchAlbumTracks('1440857781')
      expect(result.album.album).toBe('Solo EP')
      expect(result.tracks.length).toBe(1)
    })

    it('falls back to US storefront when regional album lookup returns no results', async () => {
      let callCount = 0
      setFetch(async (input: RequestInfo | URL) => {
        callCount++
        const urlStr = String(input)
        if (urlStr.includes('country=gb')) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 })
        }
        if (urlStr.includes('country=us')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'collection',
                  collectionId: 8888,
                  collectionName: 'US Album',
                  artistName: 'US Artist',
                },
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 101,
                  trackName: 'Track 1',
                  artistName: 'US Artist',
                  collectionName: 'US Album',
                },
              ],
            }),
            { status: 200 },
          )
        }
        return new Response(null, { status: 404 })
      })

      const result = await fetchAlbumTracks('8888', 'gb')
      expect(callCount).toBe(2)
      expect(result.album.title).toBe('US Album')
      expect(result.tracks.length).toBe(1)
    })

    it('throws error when no tracks found for collection', async () => {
      setFetch(
        async () =>
          new Response(
            JSON.stringify({
              results: [{ wrapperType: 'collection', collectionId: 9999 }],
            }),
            { status: 200 },
          ),
      )

      expect(fetchAlbumTracks('9999')).rejects.toThrow(
        'iTunes found no tracks for collection 9999',
      )
    })

    it('throws error on album lookup HTTP failure', async () => {
      setFetch(async () => new Response('Service Unavailable', { status: 503 }))

      expect(fetchAlbumTracks('12345')).rejects.toThrow(
        'iTunes album lookup failed (HTTP 503)',
      )
    })

    it('throws error on album lookup network timeout', async () => {
      setFetch(async () => {
        throw new Error('Socket timeout')
      })

      expect(fetchAlbumTracks('12345')).rejects.toThrow(
        'iTunes album lookup timed out',
      )
    })
  })

  describe('fetchArtistTracks', () => {
    it('resolves artist name, albums, and tracks with deduplication', async () => {
      setFetch(async (input: RequestInfo | URL) => {
        const urlStr = String(input)
        if (urlStr.includes('entity=album')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'artist',
                  artistId: 159260351,
                  artistName: 'Taylor Swift',
                },
                {
                  wrapperType: 'collection',
                  collectionId: 1001,
                  collectionName: 'Album 1',
                },
                {
                  wrapperType: 'collection',
                  collectionId: 1002,
                  collectionName: 'Album 2',
                },
              ],
            }),
            { status: 200 },
          )
        }

        if (urlStr.includes('entity=song')) {
          // Batched collection lookup
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 5001,
                  trackName: 'Song A',
                  artistName: 'Taylor Swift',
                  collectionName: 'Album 1',
                },
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 5002,
                  trackName: 'Song B',
                  artistName: 'Taylor Swift',
                  collectionName: 'Album 1',
                },
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 5001, // duplicate track across albums
                  trackName: 'Song A',
                  artistName: 'Taylor Swift',
                  collectionName: 'Album 2',
                },
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 5003,
                  trackName: 'Song C',
                  artistName: 'Taylor Swift',
                  collectionName: 'Album 2',
                },
              ],
            }),
            { status: 200 },
          )
        }

        return new Response(JSON.stringify({ results: [] }), { status: 200 })
      })

      const artistData = await fetchArtistTracks('159260351', 'us')
      expect(artistData.artistId).toBe('159260351')
      expect(artistData.artistName).toBe('Taylor Swift')
      // Deduplicated 5001, 5002, 5003 -> 3 unique tracks
      expect(artistData.tracks.length).toBe(3)
      expect(artistData.tracks.map((t) => t.id)).toEqual([
        '5001',
        '5002',
        '5003',
      ])
    })

    it('falls back to direct song lookup when artist has no album collections', async () => {
      setFetch(async (input: RequestInfo | URL) => {
        const urlStr = String(input)
        if (urlStr.includes('entity=album')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'artist',
                  artistId: 777,
                  artistName: 'Indie Artist',
                },
              ],
            }),
            { status: 200 },
          )
        }
        if (urlStr.includes('entity=song')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 9001,
                  trackName: 'Single 1',
                  artistName: 'Indie Artist',
                },
              ],
            }),
            { status: 200 },
          )
        }
        return new Response(JSON.stringify({ results: [] }), { status: 200 })
      })

      const artistData = await fetchArtistTracks('777')
      expect(artistData.artistName).toBe('Indie Artist')
      expect(artistData.tracks.length).toBe(1)
      expect(artistData.tracks[0]?.title).toBe('Single 1')
    })

    it('throws error when artist is not found in catalog', async () => {
      setFetch(
        async () =>
          new Response(JSON.stringify({ results: [] }), { status: 200 }),
      )

      expect(fetchArtistTracks('99999')).rejects.toThrow(
        'iTunes found no tracks for artist 99999',
      )
    })
  })

  describe('searchItunesCatalog', () => {
    it('searches catalog and returns mapped track items', async () => {
      setFetch(async (input: RequestInfo | URL) => {
        const urlStr = String(input)
        expect(urlStr).toContain('term=Blinding%20Lights')
        expect(urlStr).toContain('entity=song')
        expect(urlStr).toContain('limit=5')

        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'track',
                kind: 'song',
                trackId: 1499388128,
                trackName: 'Blinding Lights',
                artistName: 'The Weeknd',
                collectionName: 'After Hours',
                trackTimeMillis: 200000,
                artworkUrl100: 'https://example.com/100x100bb.jpg',
              },
            ],
          }),
          { status: 200 },
        )
      })

      const results = await searchItunesCatalog('Blinding Lights', 5)
      expect(results.length).toBe(1)
      expect(results[0].title).toBe('Blinding Lights')
      expect(results[0].artist).toBe('The Weeknd')
      expect(results[0].artworkUrl).toContain('3000x3000bb.jpg')
    })

    it('falls back to US storefront when regional search returns 0 results', async () => {
      let callCount = 0
      setFetch(async (input: RequestInfo | URL) => {
        callCount++
        const urlStr = String(input)
        if (urlStr.includes('country=fr')) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 })
        }
        if (urlStr.includes('country=us')) {
          return new Response(
            JSON.stringify({
              results: [
                {
                  wrapperType: 'track',
                  kind: 'song',
                  trackId: 7777,
                  trackName: 'Fallback Song',
                  artistName: 'Artist',
                },
              ],
            }),
            { status: 200 },
          )
        }
        return new Response(null, { status: 404 })
      })

      const results = await searchItunesCatalog('Rare Track', 5, 'fr')
      expect(callCount).toBe(2)
      expect(results.length).toBe(1)
      expect(results[0].title).toBe('Fallback Song')
    })

    it('handles fetch errors gracefully and returns empty array', async () => {
      setFetch(async () => {
        throw new Error('Network timeout')
      })

      const results = await searchItunesCatalog('Failed Query')
      expect(results).toEqual([])
    })
  })
})
