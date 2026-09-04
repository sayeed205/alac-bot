import { mkdir, unlink } from 'node:fs/promises'
import { join } from 'node:path'

import { env } from '@/env.ts'
import { debug, debugSpan } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import { fetchTrackMeta } from './itunes.ts'
import { fetchLyrics } from './lyrics.ts'
import { getMirrorEndpoint } from './manifest.ts'
import { buildTrackFilename, tagM4aFile } from './tagger.ts'
import type { TrackRipResult } from './types.ts'
import { connectAudioStreamWithWrapper } from './wrapper.ts'

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

    onProgress?.(`Connecting stream for ${meta.artist} - ${meta.title}...`)

    // Concurrently prefetch artwork and lyrics while connecting and streaming audio
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

    // 1. Resolve primary mirror endpoint from manifest / env
    let primaryMirror: { mirrorUrl: string; apiKey: string } | null = null
    try {
      primaryMirror = await getMirrorEndpoint(false, signal)
    } catch (err) {
      debug(
        'Primary mirror manifest/status lookup failed, will attempt fallback',
        {
          track_id: trackId,
          error: String(err),
        },
      )
    }

    // 2. Connect audio stream (seamlessly falling back to ALAC_WRAPPER_URL if mirror fails)
    const streamStart = Date.now()
    const { streamResp, sourceName, codec, bitDepth, sampleRate } =
      await connectAudioStreamWithWrapper({
        trackId,
        primaryMirror,
        wrapperUrl: env.ALAC_WRAPPER_URL,
        wrapperApiKey: env.ALAC_WRAPPER_API_KEY,
        signal,
        onProgress,
      })

    debug('Stream audio specs received', {
      track_id: trackId,
      source: sourceName,
      codec,
      bit_depth: bitDepth,
      sample_rate: sampleRate,
    })

    const rawFilename = `stream_${trackId}_${Date.now()}.raw`
    const tempRawPath = join(this.outputDir, rawFilename)

    const finalFilename = buildTrackFilename(meta)
    const finalPath = join(this.outputDir, finalFilename)

    // Inactivity timeout between received chunks (45s)
    // Allows arbitrarily long tracks to finish, but detects if stream freezes mid-stream
    const CHUNK_INACTIVITY_TIMEOUT_MS = 45_000
    let inactivityTimer: ReturnType<typeof setTimeout> | null = null

    const streamController = new AbortController()
    const onUserAbort = () => {
      streamController.abort(new Error('Download was cancelled'))
    }
    if (signal) {
      signal.addEventListener('abort', onUserAbort)
    }

    const resetInactivityTimer = () => {
      if (inactivityTimer) clearTimeout(inactivityTimer)
      inactivityTimer = setTimeout(() => {
        streamController.abort(
          new Error(
            `Audio stream stalled on ${sourceName}: no data received for ${CHUNK_INACTIVITY_TIMEOUT_MS / 1000}s`,
          ),
        )
      }, CHUNK_INACTIVITY_TIMEOUT_MS)
    }

    try {
      if (!streamResp.body) {
        throw new Error(`Empty audio stream body returned from ${sourceName}`)
      }

      const contentLength = Number(
        streamResp.headers.get('content-length') || 0,
      )
      let downloadedBytes = 0
      let lastProgressUpdate = 0

      const reader = streamResp.body.getReader()
      const fileSink = Bun.file(tempRawPath).writer()

      while (true) {
        if (signal?.aborted || streamController.signal.aborted) {
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

      if (signal?.aborted || streamController.signal.aborted) {
        throw new Error('Download was cancelled')
      }

      debug('Stream download completed', {
        track_id: trackId,
        source: sourceName,
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
      if (signal) {
        signal.removeEventListener('abort', onUserAbort)
      }
      await unlink(tempRawPath).catch(() => {})
    }
  }
}

export const defaultRipper: ITrackRipper = new AlacTrackRipper()
