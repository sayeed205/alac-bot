import { afterEach, describe, expect, it } from 'bun:test'

import {
  fetchPlaylistTracks,
  getAppleMusicDeveloperToken,
} from '@/modules/alac/playlist.ts'

describe('playlist module', () => {
  const originalFetch = globalThis.fetch

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  it('retrieves developer token string', async () => {
    const token = await getAppleMusicDeveloperToken()
    expect(typeof token).toBe('string')
    expect(token.length).toBeGreaterThan(20)
    expect(token.startsWith('eyJ')).toBe(true)
  })

  it('fetches playlist and tracks successfully', async () => {
    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('/v1/catalog/us/playlists/pl.test1')) {
        return new Response(
          JSON.stringify({
            data: [
              {
                id: 'pl.test1',
                attributes: {
                  name: 'Test Chill Hits',
                  curatorName: 'Apple Music Test',
                  description: { standard: 'Nice vibes' },
                },
                relationships: {
                  tracks: {
                    data: [
                      {
                        id: '1001',
                        attributes: {
                          name: 'Song A',
                          artistName: 'Artist A',
                          durationInMillis: 180000,
                        },
                      },
                      {
                        id: '1002',
                        attributes: {
                          name: 'Song B',
                          artistName: 'Artist B',
                          durationInMillis: 210000,
                        },
                      },
                    ],
                  },
                },
              },
            ],
          }),
          { status: 200 },
        )
      }
      return originalFetch(url)
    }) as unknown as typeof fetch

    const data = await fetchPlaylistTracks('pl.test1', 'us')
    expect(data.id).toBe('pl.test1')
    expect(data.title).toBe('Test Chill Hits')
    expect(data.curatorName).toBe('Apple Music Test')
    expect(data.description).toBe('Nice vibes')
    expect(data.tracks).toHaveLength(2)
    expect(data.tracks[0]).toEqual({
      id: '1001',
      title: 'Song A',
      artist: 'Artist A',
      duration: 180,
    })
    expect(data.tracks[1]).toEqual({
      id: '1002',
      title: 'Song B',
      artist: 'Artist B',
      duration: 210,
    })
  })

  it('handles multi-page pagination for large playlists', async () => {
    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('offset=2')) {
        return new Response(
          JSON.stringify({
            data: [
              {
                id: '2003',
                attributes: { name: 'Track 3', artistName: 'Art 3' },
              },
            ],
          }),
          { status: 200 },
        )
      }

      if (urlStr.includes('/v1/catalog/us/playlists/pl.pageTest')) {
        return new Response(
          JSON.stringify({
            data: [
              {
                id: 'pl.pageTest',
                attributes: { name: 'Large Playlist' },
                relationships: {
                  tracks: {
                    next: '/v1/catalog/us/playlists/pl.pageTest/tracks?offset=2',
                    data: [
                      {
                        id: '2001',
                        attributes: { name: 'Track 1', artistName: 'Art 1' },
                      },
                      {
                        id: '2002',
                        attributes: { name: 'Track 2', artistName: 'Art 2' },
                      },
                    ],
                  },
                },
              },
            ],
          }),
          { status: 200 },
        )
      }

      return originalFetch(url)
    }) as unknown as typeof fetch

    const data = await fetchPlaylistTracks('pl.pageTest', 'us')
    expect(data.tracks).toHaveLength(3)
    expect(data.tracks.map((t) => t.id)).toEqual(['2001', '2002', '2003'])
  })

  it('falls back to us storefront if regional lookup fails with 404', async () => {
    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('/v1/catalog/jp/playlists/pl.fallback')) {
        return new Response(
          JSON.stringify({ errors: [{ status: '404', code: '40400' }] }),
          { status: 404 },
        )
      }
      if (urlStr.includes('/v1/catalog/us/playlists/pl.fallback')) {
        return new Response(
          JSON.stringify({
            data: [
              {
                id: 'pl.fallback',
                attributes: { name: 'US Version' },
                relationships: {
                  tracks: {
                    data: [
                      {
                        id: '3001',
                        attributes: { name: 'Fallback Track' },
                      },
                    ],
                  },
                },
              },
            ],
          }),
          { status: 200 },
        )
      }
      return originalFetch(url)
    }) as unknown as typeof fetch

    const data = await fetchPlaylistTracks('pl.fallback', 'jp')
    expect(data.title).toBe('US Version')
    expect(data.tracks[0]?.id).toBe('3001')
  })
})
