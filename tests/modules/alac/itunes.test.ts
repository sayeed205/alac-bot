import { afterEach, describe, expect, it } from 'bun:test'

import { fetchAlbumTracks, fetchTrackMeta } from '@/modules/alac/itunes.ts'

describe('iTunes API Service', () => {
  const originalFetch = globalThis.fetch

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  describe('fetchTrackMeta', () => {
    it('fetches and maps track metadata successfully', async () => {
      globalThis.fetch = async (input: RequestInfo | URL) => {
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
      }

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
        'https://is1-ssl.mzstatic.com/image/thumb/Music125/v4/xx/1200x1200bb.jpg',
      )
    })

    it('throws error when no track is found in catalog', async () => {
      globalThis.fetch = async () =>
        new Response(JSON.stringify({ results: [] }), { status: 200 })

      expect(fetchTrackMeta('9999999999')).rejects.toThrow(
        'iTunes found no song matching track ID 9999999999',
      )
    })

    it('throws error on non-ok HTTP status', async () => {
      globalThis.fetch = async () =>
        new Response('Internal Error', { status: 502 })

      expect(fetchTrackMeta('12345')).rejects.toThrow(
        'iTunes lookup failed (HTTP 502)',
      )
    })

    it('throws error on network timeout or fetch abort', async () => {
      globalThis.fetch = async () => {
        throw new Error('Connection refused')
      }

      expect(fetchTrackMeta('12345')).rejects.toThrow(
        'iTunes track lookup timed out',
      )
    })
  })

  describe('fetchAlbumTracks', () => {
    it('fetches album collection with tracks successfully', async () => {
      globalThis.fetch = async (input: RequestInfo | URL) => {
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
      }

      const result = await fetchAlbumTracks('1440857774', 'us')
      expect(result.album.id).toBe('1440857774')
      expect(result.album.title).toBe('Shawn Mendes Deluxe')
      expect(result.album.explicit).toBe(true)
      expect(result.album.artworkUrl).toBe(
        'https://example.com/1200x1200bb.jpg',
      )
      expect(result.tracks.length).toBe(2)
      expect(result.tracks[0]?.title).toBe('Track 1')
      expect(result.tracks[1]?.title).toBe('Track 2')
    })

    it('falls back to first track when collection item is missing', async () => {
      globalThis.fetch = async () => {
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
      }

      const result = await fetchAlbumTracks('1440857774', 'us')
      expect(result.album.title).toBe('Track 1')
      expect(result.tracks.length).toBe(1)
    })

    it('throws error when no tracks found for collection', async () => {
      globalThis.fetch = async () => {
        return new Response(
          JSON.stringify({
            results: [
              {
                wrapperType: 'collection',
                collectionId: 9999,
                collectionName: 'Empty Album',
              },
            ],
          }),
          { status: 200 },
        )
      }

      expect(fetchAlbumTracks('9999')).rejects.toThrow(
        'iTunes found no tracks for collection 9999',
      )
    })

    it('throws error on album lookup HTTP failure', async () => {
      globalThis.fetch = async () =>
        new Response('Service Unavailable', { status: 503 })

      expect(fetchAlbumTracks('12345')).rejects.toThrow(
        'iTunes album lookup failed (HTTP 503)',
      )
    })

    it('throws error on album lookup network timeout', async () => {
      globalThis.fetch = async () => {
        throw new Error('Socket timeout')
      }

      expect(fetchAlbumTracks('12345')).rejects.toThrow(
        'iTunes album lookup timed out',
      )
    })
  })
})
