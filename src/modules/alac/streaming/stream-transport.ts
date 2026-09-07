import { debug, warn } from '@/utils/logger.ts'

import { mirrorPolicyManager } from './mirror-policy.ts'
import type { AudioStreamSource, ConnectStreamOptions } from './types.ts'

export interface FetchEndpointOptions {
  streamUrl: string
  apiKey?: string
  sourceName: string
  signal?: AbortSignal
  timeoutMs: number
}

export class StreamTransport {
  private readonly defaultTimeoutMs: number

  constructor(defaultTimeoutMs = 15_000) {
    this.defaultTimeoutMs = defaultTimeoutMs
  }

  /**
   * Fetches the raw audio stream from a given endpoint with custom timeout and cancellation
   */
  public async fetchEndpoint(
    options: FetchEndpointOptions,
  ): Promise<AudioStreamSource> {
    const { streamUrl, apiKey, sourceName, signal, timeoutMs } = options

    const controller = new AbortController()
    const onAbort = () => {
      controller.abort(new Error('Download was cancelled'))
    }

    if (signal) {
      if (signal.aborted) throw new Error('Download was cancelled')
      signal.addEventListener('abort', onAbort)
    }

    let timer: ReturnType<typeof setTimeout> | null = setTimeout(() => {
      controller.abort(
        new Error(
          `Stream handshake timed out after ${timeoutMs / 1000}s on ${sourceName}`,
        ),
      )
    }, timeoutMs)

    try {
      const resp = await fetch(streamUrl, {
        headers: {
          'User-Agent':
            'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
          ...(apiKey ? { 'X-API-Key': apiKey } : {}),
        },
        signal: controller.signal,
      })

      if (!resp.ok) {
        const text = await resp.text().catch(() => '')
        throw new Error(
          `HTTP ${resp.status} on ${sourceName}: ${text.slice(0, 120)}`,
        )
      }

      if (!resp.body) {
        throw new Error(`Empty body returned from ${sourceName}`)
      }

      const codec = resp.headers.get('x-codec') || 'alac'
      const rawBitDepth = resp.headers.get('x-bitdepth')
      const rawSampleRate = resp.headers.get('x-samplerate')
      const bitDepth = rawBitDepth ? Number.parseInt(rawBitDepth, 10) : 24
      const sampleRate = rawSampleRate
        ? Number.parseInt(rawSampleRate, 10)
        : 96000

      return {
        streamResp: resp,
        sourceName,
        codec,
        bitDepth,
        sampleRate,
      }
    } finally {
      if (timer) {
        clearTimeout(timer)
        timer = null
      }
      if (signal) {
        signal.removeEventListener('abort', onAbort)
      }
    }
  }

  public async connectAudioStream(
    options: ConnectStreamOptions,
  ): Promise<AudioStreamSource> {
    const {
      trackId,
      primaryMirror,
      wrapperUrl,
      wrapperApiKey,
      signal,
      onProgress,
      mirrorPolicy,
    } = options

    const policy = mirrorPolicy ?? mirrorPolicyManager
    const errors: string[] = []

    // 1. Try primary mirror if available
    if (primaryMirror) {
      try {
        debug('Attempting stream connection from primary mirror...', {
          track_id: trackId,
          mirrorUrl: primaryMirror.mirrorUrl,
        })

        const res = await this.fetchEndpoint({
          streamUrl: `${primaryMirror.mirrorUrl.replace(/\/+$/, '')}/api/stream/${trackId}`,
          apiKey: primaryMirror.apiKey,
          sourceName: `primary mirror (${new URL(primaryMirror.mirrorUrl).hostname})`,
          signal,
          timeoutMs: this.defaultTimeoutMs,
        })
        policy.recordSuccess()
        return res
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err)
        errors.push(`Primary mirror failed: ${msg}`)
        if (!signal?.aborted) {
          policy.recordFailure(msg)
        }
        warn('Primary mirror failed, checking for wrapper fallback...', {
          track_id: trackId,
          error: msg,
        })
      }
    }

    // 2. Check if wrapper URL is available
    const cleanWrapper = wrapperUrl?.trim().replace(/\/+$/, '')
    if (!cleanWrapper) {
      throw new Error(
        `Audio streaming failed and no wrapper URL is configured. Errors: ${errors.join('; ')}`,
      )
    }

    onProgress?.(
      'Primary mirror unavailable. Connecting to fallback wrapper...',
    )
    debug('Attempting stream connection from wrapper URL...', {
      track_id: trackId,
      wrapperUrl: cleanWrapper,
    })

    // Candidates to probe on wrapper / fallback endpoint
    const candidateUrls = [
      `${cleanWrapper}/api/stream/${trackId}`,
      `${cleanWrapper}/stream/${trackId}`,
    ]

    for (const candidateUrl of candidateUrls) {
      try {
        if (signal?.aborted) {
          throw new Error('Download was cancelled')
        }

        return await this.fetchEndpoint({
          streamUrl: candidateUrl,
          apiKey: wrapperApiKey,
          sourceName: `wrapper (${cleanWrapper})`,
          signal,
          timeoutMs: this.defaultTimeoutMs,
        })
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err)
        errors.push(`Wrapper candidate (${candidateUrl}) failed: ${msg}`)
        debug('Wrapper candidate endpoint failed', {
          candidate: candidateUrl,
          error: msg,
        })
      }
    }

    throw new Error(
      `Failed to stream audio from all sources. All streaming endpoints failed for track ${trackId}. Errors: ${errors.join('; ')}`,
    )
  }
}

export const streamTransport = new StreamTransport()
