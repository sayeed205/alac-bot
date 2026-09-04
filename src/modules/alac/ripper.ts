import { mkdir, unlink } from 'node:fs/promises'
import { join } from 'node:path'

import { debug, debugSpan, error } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import { fetchTrackMeta } from './itunes.ts'
import { fetchLyrics } from './lyrics.ts'
import { getMirrorEndpoint } from './manifest.ts'
import { buildTrackFilename, tagM4aFile } from './tagger.ts'
import type { TrackRipResult } from './types.ts'

export type { TrackRipResult }

export interface ITrackRipper {
  rip(
    trackId: string,
    onProgress?: (status: string) => void,
    storefront?: string,
    signal?: AbortSignal,
  ): Promise<TrackRipResult>
}

export class FakeTrackRipper implements ITrackRipper {
  constructor(private mockResult?: Partial<TrackRipResult>) {}

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
    _storefront?: string,
    signal?: AbortSignal,
  ): Promise<TrackRipResult> {
    if (signal?.aborted) {
      throw new Error('Download was cancelled')
    }
    onProgress?.('Downloading metadata...')
    await new Promise((r) => setTimeout(r, 10))
    if (signal?.aborted) {
      throw new Error('Download was cancelled')
    }
    onProgress?.('Fetching ALAC stream...')
    await new Promise((r) => setTimeout(r, 10))

    return {
      filePath: this.mockResult?.filePath || `/tmp/mock_${trackId}.m4a`,
      title: this.mockResult?.title || `Mock Song ${trackId}`,
      artist: this.mockResult?.artist || 'Mock Artist',
      album: this.mockResult?.album || 'Mock Album',
      duration: this.mockResult?.duration || 210,
      codec: 'alac',
      bitDepth: 24,
      sampleRate: 96000,
      genre: this.mockResult?.genre || 'Pop',
      releaseDate: this.mockResult?.releaseDate || '2023-01-01',
      trackNumber: this.mockResult?.trackNumber || 1,
      trackCount: this.mockResult?.trackCount || 10,
    }
  }
}

export class AlacTrackRipper implements ITrackRipper {
  private readonly outputDir: string

  constructor(outputDir?: string) {
    this.outputDir = outputDir || join(process.cwd(), 'bot-data', 'downloads')
  }

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
    storefront?: string,
    signal?: AbortSignal,
  ): Promise<TrackRipResult> {
    using _ = debugSpan('ripper', { track_id: trackId, storefront }).enter()
    const ripStart = Date.now()

    if (signal?.aborted) {
      throw new Error('Download was cancelled')
    }

    await mkdir(this.outputDir, { recursive: true })

    onProgress?.('Fetching track metadata...')
    const meta = await fetchTrackMeta(trackId, storefront)

    if (signal?.aborted) {
      throw new Error('Download was cancelled')
    }

    onProgress?.(`Connecting mirror for ${meta.artist} - ${meta.title}...`)
    const { mirrorUrl, apiKey } = await getMirrorEndpoint(false, signal)

    // Concurrently prefetch artwork and lyrics while streaming audio
    const lyricsPromise = fetchLyrics(trackId, {
      title: meta.title,
      artist: meta.artist,
      album: meta.album,
      duration: meta.duration,
    })
      .then((l) => {
        debug('Lyrics prefetch completed', {
          track_id: trackId,
          found: Boolean(l),
        })
        return l
      })
      .catch((err) => {
        debug('Lyrics prefetch failed', {
          track_id: trackId,
          error: String(err),
        })
        return null
      })

    const artworkPromise = (async () => {
      if (!meta.artworkUrl) return null
      try {
        const artworkSignal = signal
          ? AbortSignal.any([signal, AbortSignal.timeout(15_000)])
          : AbortSignal.timeout(15_000)

        const r = await fetch(meta.artworkUrl, {
          headers: {
            'User-Agent': 'AlacBot/1.0',
          },
          signal: artworkSignal,
        })
        if (!r.ok) return null
        const buf = new Uint8Array(await r.arrayBuffer())
        debug('Artwork prefetch completed', {
          track_id: trackId,
          size_bytes: buf.byteLength,
        })
        return buf
      } catch (err) {
        debug('Artwork prefetch timed out/failed', {
          track_id: trackId,
          error: String(err),
        })
        return null
      }
    })()

    // Stream download from mirror
    const streamUrl = `${mirrorUrl}/api/stream/${trackId}`
    debug('Initiating mirror stream connection...', {
      track_id: trackId,
      streamUrl,
    })

    const streamStart = Date.now()
    const streamController = new AbortController()

    const onUserAbort = () => {
      streamController.abort(new Error('Download was cancelled'))
    }
    if (signal) {
      if (signal.aborted) {
        throw new Error('Download was cancelled')
      }
      signal.addEventListener('abort', onUserAbort)
    }

    // Step 1: Initial Connection Timeout (30s)
    // If mirror backend hangs or crashes on handshake, abort before hanging forever
    let connectionTimer: ReturnType<typeof setTimeout> | null = setTimeout(
      () => {
        streamController.abort(
          new Error(
            'Mirror server did not respond within 30s (server down or overloaded)',
          ),
        )
      },
      30_000,
    )

    let streamResp: Response
    try {
      try {
        streamResp = await fetch(streamUrl, {
          headers: {
            'User-Agent':
              'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
            'X-API-Key': apiKey,
          },
          signal: streamController.signal,
        })
      } catch (err: unknown) {
        const elapsed = Date.now() - streamStart
        error('Mirror stream connection timed out or failed', {
          track_id: trackId,
          elapsed_ms: elapsed,
          error: err instanceof Error ? err.message : String(err),
        })
        throw new Error(
          `Failed to connect to mirror stream after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
        )
      } finally {
        if (connectionTimer) {
          clearTimeout(connectionTimer)
          connectionTimer = null
        }
      }

      if (!streamResp.ok) {
        const body = await streamResp.text().catch(() => '')
        error('Mirror stream returned HTTP error', {
          track_id: trackId,
          status: streamResp.status,
          body,
        })
        throw new Error(
          `Mirror streaming failed with HTTP ${streamResp.status}: ${body}`,
        )
      }

      if (!streamResp.body) {
        throw new Error('Mirror stream response body is null')
      }

      // Audio format headers
      const codec = streamResp.headers.get('x-codec') || 'alac'
      const rawBitDepth = streamResp.headers.get('x-bitdepth')
      const rawSampleRate = streamResp.headers.get('x-samplerate')
      const bitDepth = rawBitDepth ? Number.parseInt(rawBitDepth, 10) : 24
      const sampleRate = rawSampleRate
        ? Number.parseInt(rawSampleRate, 10)
        : 96000

      debug('Stream audio specs received', {
        track_id: trackId,
        codec,
        bit_depth: bitDepth,
        sample_rate: sampleRate,
      })

      const rawFilename = `stream_${trackId}_${Date.now()}.raw`
      const tempRawPath = join(this.outputDir, rawFilename)

      const finalFilename = buildTrackFilename(meta)
      const finalPath = join(this.outputDir, finalFilename)

      // Step 2: Inactivity timeout between received chunks (45s)
      // Allows arbitrarily long tracks to finish, but detects if mirror freezes mid-stream
      const CHUNK_INACTIVITY_TIMEOUT_MS = 45_000
      let inactivityTimer: ReturnType<typeof setTimeout> | null = null

      const resetInactivityTimer = () => {
        if (inactivityTimer) clearTimeout(inactivityTimer)
        inactivityTimer = setTimeout(() => {
          streamController.abort(
            new Error(
              `Mirror stream stalled: no data received for ${CHUNK_INACTIVITY_TIMEOUT_MS / 1000}s`,
            ),
          )
        }, CHUNK_INACTIVITY_TIMEOUT_MS)
      }

      try {
        const contentLength = Number(
          streamResp.headers.get('content-length') || 0,
        )
        let downloadedBytes = 0
        let lastProgressUpdate = 0

        const reader = streamResp.body.getReader()
        const fileSink = Bun.file(tempRawPath).writer()

        while (true) {
          if (signal?.aborted) {
            await reader.cancel()
            await fileSink.end()
            throw new Error('Download was cancelled')
          }

          resetInactivityTimer()

          const { done, value } = await reader.read()
          if (done) break

          fileSink.write(value)
          downloadedBytes += value.length

          const now = Date.now()
          if (now - lastProgressUpdate > 1000 && onProgress) {
            lastProgressUpdate = now
            const progressStr = formatByteProgress(
              downloadedBytes,
              contentLength > 0 ? contentLength : 0,
            )
            onProgress(`Downloading lossless audio: ${progressStr}`)
          }
        }

        if (inactivityTimer) {
          clearTimeout(inactivityTimer)
          inactivityTimer = null
        }
        await fileSink.end()

        if (signal?.aborted) {
          throw new Error('Download was cancelled')
        }

        debug('Stream download completed', {
          track_id: trackId,
          downloaded_bytes: downloadedBytes,
          stream_duration_ms: Date.now() - streamStart,
        })

        onProgress?.('Tagging and embedding lossless artwork...')
        const tagStart = Date.now()
        const [coverBuffer, lyrics] = await Promise.all([
          artworkPromise,
          lyricsPromise,
        ])

        if (signal?.aborted) {
          throw new Error('Download was cancelled')
        }

        await tagM4aFile({
          rawAudioPath: tempRawPath,
          outputPath: finalPath,
          meta,
          coverBuffer,
          lyrics,
        })

        debug('Tagging finished', {
          track_id: trackId,
          has_lyrics: Boolean(lyrics),
          has_cover: Boolean(coverBuffer),
          tag_duration_ms: Date.now() - tagStart,
          total_duration_ms: Date.now() - ripStart,
        })

        return {
          filePath: finalPath,
          title: meta.title,
          artist: meta.artist,
          album: meta.album,
          duration: meta.duration,
          codec,
          bitDepth,
          sampleRate,
          genre: meta.genre || 'Unknown',
          releaseDate: meta.releaseDate || '',
          trackNumber: meta.trackNumber ?? 1,
          trackCount: meta.trackCount ?? 1,
        }
      } finally {
        if (inactivityTimer) {
          clearTimeout(inactivityTimer)
        }
        await unlink(tempRawPath).catch(() => {})
      }
    } finally {
      if (connectionTimer) {
        clearTimeout(connectionTimer)
      }
      if (signal) {
        signal.removeEventListener('abort', onUserAbort)
      }
    }
  }
}

export const defaultRipper: ITrackRipper = new AlacTrackRipper()
