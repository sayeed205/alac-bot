import { describe, expect, it } from 'bun:test'

import { extractBatchItems, parseAlacInput } from '@/modules/alac/parser.ts'

describe('parseAlacInput', () => {
  it('extracts track ID and storefront from standard album link with ?i= parameter', () => {
    const input =
      '/alac https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1193701400', type: 'track', storefront: 'us' }],
      trackId: '1193701400',
      force: false,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'us',
    })
  })

  it('extracts track ID when -f flag is provided', () => {
    const input =
      '/alac https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400 -f'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1193701400', type: 'track', storefront: 'us' }],
      trackId: '1193701400',
      force: true,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'us',
    })
  })

  it('extracts track ID when --force flag is provided in front', () => {
    const input =
      '/alac --force https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1193701400', type: 'track', storefront: 'us' }],
      trackId: '1193701400',
      force: true,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'us',
    })
  })

  it('extracts track ID from direct /song/ link with regional storefront', () => {
    const input = '/alac https://music.apple.com/in/song/tum-hi-ho/1122334455'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1122334455', type: 'track', storefront: 'in' }],
      trackId: '1122334455',
      force: false,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'in',
    })
  })

  it('extracts album ID from direct /album/ link without ?i=', () => {
    const input =
      '/alac https://music.apple.com/us/album/blinding-lights/1499378108'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1499378108', type: 'album', storefront: 'us' }],
      trackId: '1499378108',
      force: false,
      isAlbum: true,
      isPlaylist: false,
      storefront: 'us',
    })
  })

  it('extracts playlist ID from Apple Music playlist link', () => {
    const input =
      '/alac https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [
        {
          id: 'pl.f4d106fed2bd41149aaacabb233eb5eb',
          type: 'playlist',
          storefront: 'us',
        },
      ],
      trackId: 'pl.f4d106fed2bd41149aaacabb233eb5eb',
      force: false,
      isAlbum: false,
      isPlaylist: true,
      storefront: 'us',
    })
  })

  it('extracts user-curated playlist with pl.u- prefix', () => {
    const input =
      '/batch https://music.apple.com/playlist/chill-vibes/pl.u-76oNke3FvPyK8r'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: 'pl.u-76oNke3FvPyK8r', type: 'playlist' }],
      trackId: 'pl.u-76oNke3FvPyK8r',
      force: false,
      isAlbum: false,
      isPlaylist: true,
    })
  })

  it('extracts bare track ID without storefront', () => {
    const input = '/alac 1440841730'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: '1440841730', type: 'track' }],
      trackId: '1440841730',
      force: false,
      isAlbum: false,
      isPlaylist: false,
    })
  })

  it('extracts bare playlist ID', () => {
    const input = '/dl pl.f4d106fed2bd41149aaacabb233eb5eb'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      items: [{ id: 'pl.f4d106fed2bd41149aaacabb233eb5eb', type: 'playlist' }],
      trackId: 'pl.f4d106fed2bd41149aaacabb233eb5eb',
      force: false,
      isAlbum: false,
      isPlaylist: true,
    })
  })

  it('extracts multiple items from multi-link command', () => {
    const input =
      '/batch https://music.apple.com/us/album/song1/1000?i=1111 https://music.apple.com/us/album/album2/2222'
    const res = parseAlacInput(input)
    expect(res?.items).toHaveLength(2)
    expect(res?.items[0]).toEqual({
      id: '1111',
      type: 'track',
      storefront: 'us',
    })
    expect(res?.items[1]).toEqual({
      id: '2222',
      type: 'album',
      storefront: 'us',
    })
  })

  it('extracts from replied message if command has no arguments', () => {
    const replyText =
      'Check this song https://music.apple.com/jp/album/song/1000?i=2000 it is awesome'
    const res = parseAlacInput('/alac', replyText)
    expect(res).toEqual({
      items: [{ id: '2000', type: 'track', storefront: 'jp' }],
      trackId: '2000',
      force: false,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'jp',
    })
  })

  it('extracts from replied message with -f on command', () => {
    const replyText = 'https://music.apple.com/us/album/song/1000?i=2000'
    const res = parseAlacInput('/alac -f', replyText)
    expect(res).toEqual({
      items: [{ id: '2000', type: 'track', storefront: 'us' }],
      trackId: '2000',
      force: true,
      isAlbum: false,
      isPlaylist: false,
      storefront: 'us',
    })
  })

  it('returns null for invalid input', () => {
    expect(parseAlacInput('/alac')).toBeNull()
    expect(parseAlacInput('/alac not_a_link')).toBeNull()
  })
})

describe('extractBatchItems', () => {
  it('extracts multiple links and ignores comments and blank lines', () => {
    const fileContent = `
# My Queue of songs
https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400

// Album link
https://music.apple.com/us/album/blinding-lights/1499378108

# Playlist
https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb
1440841730
`
    const items = extractBatchItems(fileContent)
    expect(items).toEqual([
      { id: '1193701400', type: 'track', storefront: 'us' },
      { id: '1499378108', type: 'album', storefront: 'us' },
      {
        id: 'pl.f4d106fed2bd41149aaacabb233eb5eb',
        type: 'playlist',
        storefront: 'us',
      },
      { id: '1440841730', type: 'track' },
    ])
  })

  it('deduplicates identical entries', () => {
    const fileContent = `
1440841730
1440841730
https://music.apple.com/us/album/song/1?i=1440841730
`
    const items = extractBatchItems(fileContent)
    expect(items).toHaveLength(1)
    expect(items[0]?.id).toBe('1440841730')
  })
})
