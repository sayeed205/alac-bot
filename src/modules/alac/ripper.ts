import { mkdir, unlink } from 'node:fs/promises'
import { join } from 'node:path'

import { debug, debugSpan, error, info, infoSpan } from '@/utils/logger.ts'
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
  ): Promise<TrackRipResult>
}

export class FakeTrackRipper implements ITrackRipper {
  constructor(private mockResult?: Partial<TrackRipResult>) {}

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
  ): Promise<TrackRipResult> {
    onProgress?.('Downloading metadata...')
    await new Promise((r) => setTimeout(r, 10))
    onProgress?.('Fetching ALAC stream...')
    await new Promise((r) => setTimeout(r, 10))

    return {
      filePath: this.mockResult?.filePath || `/tmp/mock_${trackId}.m4a`,
      title: this.mockResult?.title || `Mock Song ${trackId}`,
      artist: this.mockResult?.artist || 'Mock Artist',
      album: this.mockResult?.album || 'Mock Album',
      duration: this.mockResult?.duration || 210,
      codec: 'alac',
      bitDepth: '24',
      sampleRate: '96000',
    }
  }
}

export class AlacTrackRipper implements ITrackRipper {
  private outputDir: string

  constructor(outputDir?: string) {
    this.outputDir = outputDir || join(process.cwd(), 'bot-data', 'downloads')
  }

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
  ): Promise<TrackRipResult> {
    using _ = debugSpan('ripper', { track_id: trackId }).enter()
    const ripStart = Date.now()

    await mkdir(this.outputDir, { recursive: true })

    onProgress?.('Fetching track metadata...')
    const meta = await fetchTrackMeta(trackId)

    onProgress?.(`Connecting mirror for ${meta.artist} - ${meta.title}...`)
    const { mirrorUrl, apiKey } = await getMirrorEndpoint()

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

    const artworkPromise = meta.artworkUrl
      ? fetch(meta.artworkUrl, {
          headers: {
            'User-Agent': 'AlacBot/1.0',
          },
          signal: AbortSignal.timeout(15_000),
        })
          .then(async (r) => {
            if (!r.ok) return null
            const buf = new Uint8Array(await r.arrayBuffer())
            debug('Artwork prefetch completed', {
              track_id: trackId,
              size_bytes: buf.byteLength,
            })
            return buf
          })
          .catch((err) => {
            debug('Artwork prefetch timed out/failed', {
              track_id: trackId,
              error: String(err),
            })
            return null
          })
      : Promise.resolve(null)

    // Stream download from mirror
    const streamUrl = `${mirrorUrl}/api/stream/${trackId}`
    debug('Initiating mirror stream connection...', {
      track_id: trackId,
      streamUrl,
    })

    const streamStart = Date.now()
    let streamResp: Response
    try {
      streamResp = await fetch(streamUrl, {
        headers: {
          'User-Agent':
            'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
          'X-API-Key': apiKey,
        },
        signal: AbortSignal.timeout(120_000),
      })
    } catch (err: unknown) {
      const elapsed = Date.now() - streamStart
      error('Mirror stream connection timed out or failed', {
        track_id: trackId,
        streamUrl,
        elapsed_ms: elapsed,
        error: err instanceof Error ? err.message : String(err),
      })
      throw new Error(
        `Mirror audio stream failed after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`,
      )
    }

    if (!streamResp.ok) {
      const errBody = await streamResp.text().catch(() => '')
      error('Mirror stream HTTP failure', {
        track_id: trackId,
        status: streamResp.status,
        body: errBody.slice(0, 200),
      })
      throw new Error(
        `Mirror stream failed (${streamResp.status}): ${errBody.slice(0, 200)}`,
      )
    }

    const codec = streamResp.headers.get('x-codec') || 'alac'
    const bitDepth = streamResp.headers.get('x-bitdepth') || '16'
    const sampleRate = streamResp.headers.get('x-samplerate') || '44100'
    const contentLength = Number(streamResp.headers.get('content-length')) || 0

    debug('Mirror stream connected', {
      track_id: trackId,
      connect_duration_ms: Date.now() - streamStart,
      codec,
      bit_depth: bitDepth,
      sample_rate: sampleRate,
      content_length: contentLength,
    })

    const tempRawPath = join(this.outputDir, `raw_${trackId}_${Date.now()}.m4a`)
    const finalFilename = buildTrackFilename(meta)
    const finalPath = join(this.outputDir, finalFilename)

    const reader = streamResp.body?.getReader()
    if (!reader) {
      error('Mirror returned empty stream body', { track_id: trackId })
      throw new Error('Mirror returned response without body stream')
    }

    const fileSink = Bun.file(tempRawPath).writer()
    let downloadedBytes = 0
    let lastReport = 0

    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        if (value) {
          fileSink.write(value)
          downloadedBytes += value.byteLength

          const now = Date.now()
          if (now - lastReport > 700) {
            lastReport = now
            if (contentLength > 0) {
              const progressText = formatByteProgress(
                downloadedBytes,
                contentLength,
                10,
              )
              onProgress?.(`Streaming ALAC:<br/><code>${progressText}</code>`)
            } else {
              const mb = (downloadedBytes / (1024 * 1024)).toFixed(1)
              onProgress?.(`Streaming ALAC: ${mb} MB`)
            }
          }
        }
      }
      await fileSink.end()

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
      }
    } finally {
      await unlink(tempRawPath).catch(() => {})
    }
  }
}

export const defaultRipper: ITrackRipper = new AlacTrackRipper()
