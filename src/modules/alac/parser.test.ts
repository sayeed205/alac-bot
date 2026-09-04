import { describe, expect, it } from 'bun:test'

import { parseAlacInput } from '@/modules/alac/parser.ts'

describe('parseAlacInput', () => {
  it('extracts track ID from standard album link with ?i= parameter', () => {
    const input =
      '/alac https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1193701400',
      force: false,
      isAlbum: false,
    })
  })

  it('extracts track ID when -f flag is provided', () => {
    const input =
      '/alac https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400 -f'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1193701400',
      force: true,
      isAlbum: false,
    })
  })

  it('extracts track ID when --force flag is provided in front', () => {
    const input =
      '/alac --force https://music.apple.com/us/album/shape-of-you/1193701079?i=1193701400'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1193701400',
      force: true,
      isAlbum: false,
    })
  })

  it('extracts track ID from direct /song/ link', () => {
    const input = '/alac https://music.apple.com/in/song/tum-hi-ho/1122334455'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1122334455',
      force: false,
      isAlbum: false,
    })
  })

  it('extracts album ID from direct /album/ link without ?i=', () => {
    const input =
      '/alac https://music.apple.com/us/album/blinding-lights/1499378108'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1499378108',
      force: false,
      isAlbum: true,
    })
  })

  it('extracts bare track ID', () => {
    const input = '/alac 1440841730'
    const res = parseAlacInput(input)
    expect(res).toEqual({
      trackId: '1440841730',
      force: false,
      isAlbum: false,
    })
  })

  it('extracts from replied message if command has no arguments', () => {
    const replyText =
      'Check this song https://music.apple.com/us/album/song/1000?i=2000 it is awesome'
    const res = parseAlacInput('/alac', replyText)
    expect(res).toEqual({
      trackId: '2000',
      force: false,
      isAlbum: false,
    })
  })

  it('extracts from replied message with -f on command', () => {
    const replyText = 'https://music.apple.com/us/album/song/1000?i=2000'
    const res = parseAlacInput('/alac -f', replyText)
    expect(res).toEqual({
      trackId: '2000',
      force: true,
      isAlbum: false,
    })
  })

  it('returns null for invalid input', () => {
    expect(parseAlacInput('/alac')).toBeNull()
    expect(parseAlacInput('/alac not_a_link')).toBeNull()
  })
})
