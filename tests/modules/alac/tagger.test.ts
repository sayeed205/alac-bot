import { afterEach, describe, expect, it, mock } from 'bun:test'

import {
  buildTrackFilename,
  sanitizeFilename,
  tagM4aFile,
} from '@/modules/alac/tagger.ts'
import type { AppleTrackMetadata } from '@/modules/alac/types.ts'

describe('M4A Metadata Tagger', () => {
  const originalSpawn = Bun.spawn

  afterEach(() => {
    Bun.spawn = originalSpawn
  })

  it('sanitizes illegal filename characters and handles fallback', () => {
    expect(sanitizeFilename('Song: "Super" / <Title>? *|')).toBe(
      'Song_ _Super_ _ _Title__ __',
    )
    expect(sanitizeFilename('')).toBe('track')
    expect(sanitizeFilename('   ')).toBe('track')
  })

  it('builds formatted track filename with explicit flag', () => {
    const metaClean: AppleTrackMetadata = {
      id: '1',
      title: 'Clean Song',
      artist: 'Artist',
      album: 'Album',
      albumArtist: 'Artist',
      duration: 180,
      explicit: false,
      trackNumber: 3,
    }
    expect(buildTrackFilename(metaClean)).toBe(
      '03. Clean Song - Artist [ALAC].m4a',
    )

    const metaExplicit: AppleTrackMetadata = {
      id: '2',
      title: 'Explicit Song',
      artist: 'Artist',
      album: 'Album',
      albumArtist: 'Artist',
      duration: 180,
      explicit: true,
      trackNumber: 12,
    }
    expect(buildTrackFilename(metaExplicit)).toBe(
      '12. Explicit Song - Artist [E] [ALAC].m4a',
    )
  })

  it('tags m4a file with full metadata, cover artwork, and lyrics', async () => {
    let capturedArgs: string[] = []

    Bun.spawn = mock((args: string[]) => {
      capturedArgs = args
      return {
        exited: Promise.resolve(0),
        stdout: new ReadableStream(),
        stderr: new ReadableStream(),
      } as unknown as ReturnType<typeof Bun.spawn>
    }) as unknown as typeof Bun.spawn

    const meta: AppleTrackMetadata = {
      id: '100',
      title: 'Track Title',
      artist: 'Artist Name',
      album: 'Album Name',
      albumArtist: 'Album Artist',
      releaseDate: '2023-01-01',
      genre: 'Rock',
      composer: 'Composer Name',
      trackNumber: 2,
      trackCount: 10,
      discNumber: 1,
      discCount: 1,
      duration: 200,
      explicit: false,
    }

    const coverBuffer = new Uint8Array([0xff, 0xd8, 0xff])
    const outPath = '/tmp/test_output.m4a'

    const result = await tagM4aFile({
      rawAudioPath: '/tmp/test_raw.m4a',
      outputPath: outPath,
      meta,
      coverBuffer,
      lyrics: '[00:10.00]Lyric line',
    })

    expect(result).toBe(outPath)
    expect(capturedArgs).toContain('ffmpeg')
    expect(capturedArgs).toContain('title=Track Title')
    expect(capturedArgs).toContain('artist=Artist Name')
    expect(capturedArgs).toContain('album=Album Name')
    expect(capturedArgs).toContain('album_artist=Album Artist')
    expect(capturedArgs).toContain('date=2023-01-01')
    expect(capturedArgs).toContain('genre=Rock')
    expect(capturedArgs).toContain('composer=Composer Name')
    expect(capturedArgs).toContain('track=2/10')
    expect(capturedArgs).toContain('disc=1/1')
    expect(capturedArgs).toContain('lyrics=[00:10.00]Lyric line')
  })

  it('tags m4a file without cover or lyrics', async () => {
    let capturedArgs: string[] = []

    Bun.spawn = mock((args: string[]) => {
      capturedArgs = args
      return {
        exited: Promise.resolve(0),
        stdout: new ReadableStream(),
        stderr: new ReadableStream(),
      } as unknown as ReturnType<typeof Bun.spawn>
    }) as unknown as typeof Bun.spawn

    const meta: AppleTrackMetadata = {
      id: '101',
      title: 'Minimal Song',
      artist: 'Artist',
      album: 'Album',
      albumArtist: 'Artist',
      duration: 150,
      explicit: false,
    }

    const outPath = '/tmp/test_min.m4a'
    const result = await tagM4aFile({
      rawAudioPath: '/tmp/test_raw_min.m4a',
      outputPath: outPath,
      meta,
    })

    expect(result).toBe(outPath)
    expect(capturedArgs).toContain('-c')
    expect(capturedArgs).toContain('copy')
    expect(capturedArgs).not.toContain('attached_pic')
  })

  it('throws error when FFmpeg process fails', async () => {
    Bun.spawn = mock(() => {
      return {
        exited: Promise.resolve(1),
        stdout: new ReadableStream(),
        stderr: new Response('Invalid stream data').body,
      } as unknown as ReturnType<typeof Bun.spawn>
    }) as unknown as typeof Bun.spawn

    const meta: AppleTrackMetadata = {
      id: '102',
      title: 'Broken Song',
      artist: 'Artist',
      album: 'Album',
      albumArtist: 'Artist',
      duration: 100,
      explicit: false,
    }

    expect(
      tagM4aFile({
        rawAudioPath: '/tmp/broken.m4a',
        outputPath: '/tmp/broken_out.m4a',
        meta,
      }),
    ).rejects.toThrow('FFmpeg tagging failed')
  })
})
