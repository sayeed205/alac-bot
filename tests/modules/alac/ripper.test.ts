import { describe, expect, it } from 'bun:test'

import {
  AlacTrackRipper,
  abortableSleep,
  type TrackRipResult,
} from '@/modules/alac/ripper.ts'
import { buildTrackFilename, sanitizeFilename } from '@/modules/alac/tagger.ts'
import type { AppleTrackMetadata } from '@/modules/alac/types.ts'

class TestableAlacTrackRipper extends AlacTrackRipper {
  public attempts = 0
  public customRipOnce?: (attempt: number) => Promise<TrackRipResult>

  protected override async ripOnce(
    _trackId: string,
    _onProgress?: (status: string) => void,
    _storefront?: string,
    signal?: AbortSignal,
  ): Promise<TrackRipResult> {
    this.attempts++
    if (signal?.aborted) {
      throw new Error('Download was cancelled')
    }
    if (this.customRipOnce) {
      return await this.customRipOnce(this.attempts)
    }
    return {
      filePath: '/tmp/test.m4a',
      title: 'Song',
      artist: 'Artist',
      album: 'Album',
      duration: 180,
      codec: 'alac',
      bitDepth: 24,
      sampleRate: 96000,
      genre: 'Pop',
      releaseDate: '2024-01-01',
      trackNumber: 1,
      trackCount: 1,
    }
  }
}

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

describe('abortableSleep', () => {
  it('resolves after specified milliseconds', async () => {
    const start = Date.now()
    await abortableSleep(20)
    expect(Date.now() - start).toBeGreaterThanOrEqual(15)
  })

  it('rejects immediately if signal is already aborted', async () => {
    const controller = new AbortController()
    controller.abort()
    expect(abortableSleep(1000, controller.signal)).rejects.toThrow(
      'Download was cancelled',
    )
  })

  it('rejects immediately when signal aborts during sleep', async () => {
    const controller = new AbortController()
    const sleepPromise = abortableSleep(5000, controller.signal)
    setTimeout(() => controller.abort(), 20)
    expect(sleepPromise).rejects.toThrow('Download was cancelled')
  })
})

describe('AlacTrackRipper Retry with Exponential Backoff', () => {
  it('succeeds on first attempt without retrying', async () => {
    const ripper = new TestableAlacTrackRipper('/tmp', 3, 10)
    const result = await ripper.rip('12345')
    expect(result.title).toBe('Song')
    expect(ripper.attempts).toBe(1)
  })

  it('retries on transient failure and succeeds on subsequent attempt', async () => {
    const ripper = new TestableAlacTrackRipper('/tmp', 3, 10)
    const progressMessages: string[] = []

    ripper.customRipOnce = async (attempt: number) => {
      if (attempt < 3) {
        throw new Error(`Transient network glitch on attempt ${attempt}`)
      }
      return {
        filePath: '/tmp/recovered.m4a',
        title: 'Recovered Song',
        artist: 'Artist',
        album: 'Album',
        duration: 200,
        codec: 'alac',
        bitDepth: 24,
        sampleRate: 96000,
        genre: 'Rock',
        releaseDate: '2024-01-01',
        trackNumber: 1,
        trackCount: 1,
      }
    }

    const result = await ripper.rip('12345', (msg) => {
      progressMessages.push(msg)
    })

    expect(result.title).toBe('Recovered Song')
    expect(ripper.attempts).toBe(3) // Initial + 2 retries
    expect(progressMessages.length).toBe(2)
    expect(progressMessages[0]).toContain(
      '⚠️ Rip failed, retrying (attempt 1/3)',
    )
    expect(progressMessages[1]).toContain(
      '⚠️ Rip failed, retrying (attempt 2/3)',
    )
  })

  it('exhausts retries and rethrows the final error', async () => {
    const ripper = new TestableAlacTrackRipper('/tmp', 3, 5)
    const progressMessages: string[] = []

    ripper.customRipOnce = async () => {
      throw new Error('Persistent mirror 503 error')
    }

    expect(
      ripper.rip('99999', (msg) => {
        progressMessages.push(msg)
      }),
    ).rejects.toThrow('Persistent mirror 503 error')

    expect(ripper.attempts).toBe(4) // 1 initial + 3 retries = 4 attempts total
    expect(progressMessages.length).toBe(3) // 3 retry notifications
    expect(progressMessages[2]).toContain(
      '⚠️ Rip failed, retrying (attempt 3/3)',
    )
  })

  it('does not retry when cancelled by signal', async () => {
    const ripper = new TestableAlacTrackRipper('/tmp', 3, 5)
    const controller = new AbortController()
    controller.abort()

    expect(
      ripper.rip('12345', undefined, undefined, controller.signal),
    ).rejects.toThrow('Download was cancelled')

    expect(ripper.attempts).toBe(0)
  })

  it('aborts immediately during backoff sleep when user cancels', async () => {
    const ripper = new TestableAlacTrackRipper('/tmp', 3, 5000)
    const controller = new AbortController()

    ripper.customRipOnce = async () => {
      throw new Error('Stream disconnected')
    }

    const ripPromise = ripper.rip(
      '12345',
      undefined,
      undefined,
      controller.signal,
    )

    // Abort after 50ms (during the 5s backoff sleep)
    setTimeout(() => {
      controller.abort()
    }, 50)

    const start = Date.now()
    expect(ripPromise).rejects.toThrow('Download was cancelled')
    expect(Date.now() - start).toBeLessThan(500) // Did not wait full 5s
    expect(ripper.attempts).toBe(1)
  })
})
