import { describe, expect, it } from 'bun:test'

import { buildTrackFilename, sanitizeFilename } from '@/modules/alac/tagger.ts'
import type { AppleTrackMetadata } from '@/modules/alac/types.ts'

describe('Native Ripper Helpers', () => {
  it('sanitizes illegal characters from filenames', () => {
    expect(sanitizeFilename('AC/DC: Back in Black?')).toBe(
      'AC_DC_ Back in Black_',
    )
    expect(sanitizeFilename('Song <feat. Artist> *remix*')).toBe(
      'Song _feat. Artist_ _remix_',
    )
  })

  it('builds formatted m4a filename with track number and lossless tag', () => {
    const meta: AppleTrackMetadata = {
      id: '12345',
      title: 'Bohemian Rhapsody',
      artist: 'Queen',
      album: 'A Night at the Opera',
      albumArtist: 'Queen',
      trackNumber: 4,
      trackCount: 12,
      duration: 354,
      explicit: false,
    }

    const filename = buildTrackFilename(meta)
    expect(filename).toBe('04. Bohemian Rhapsody - Queen [ALAC].m4a')
  })

  it('adds [E] flag for explicit tracks', () => {
    const meta: AppleTrackMetadata = {
      id: '67890',
      title: 'Starboy',
      artist: 'The Weeknd',
      album: 'Starboy',
      albumArtist: 'The Weeknd',
      trackNumber: 1,
      duration: 230,
      explicit: true,
    }

    const filename = buildTrackFilename(meta)
    expect(filename).toBe('01. Starboy - The Weeknd [E] [ALAC].m4a')
  })
})
