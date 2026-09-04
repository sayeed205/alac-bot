import { debug, error, warn } from '@/utils/logger.ts'

export interface AudioStreamSource {
  streamResp: Response
  sourceName: string
  codec: string
  bitDepth: number
  sampleRate: number
}

interface ConnectStreamOptions {
  trackId: string
  primaryMirror: { mirrorUrl: string; apiKey: string } | null
  wrapperUrl?: string
  wrapperApiKey?: string
  signal?: AbortSignal
  onProgress?: (status: string) => void
}

/**
 * Attempts to connect to an audio stream from the primary mirror,
 * seamlessly falling back to a configured local/remote wrapper or secondary mirror URL.
 */
export async function connectAudioStreamWithWrapper(
  options: ConnectStreamOptions,
): Promise<AudioStreamSource> {
  const {
    trackId,
    primaryMirror,
    wrapperUrl,
    wrapperApiKey,
    signal,
    onProgress,
  } = options

  const errors: string[] = []

  // 1. Try primary mirror if available
  if (primaryMirror) {
    try {
      debug('Attempting stream connection from primary mirror...', {
        track_id: trackId,
        mirrorUrl: primaryMirror.mirrorUrl,
      })

      const source = await fetchStreamEndpoint({
        streamUrl: `${primaryMirror.mirrorUrl.replace(/\/+$/, '')}/api/stream/${trackId}`,
        apiKey: primaryMirror.apiKey,
        sourceName: `primary mirror (${new URL(primaryMirror.mirrorUrl).hostname})`,
        signal,
        timeoutMs: 30_000,
      })

      return source
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      errors.push(`Primary mirror failed: ${msg}`)
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

  onProgress?.('Primary mirror unavailable. Connecting to fallback wrapper...')
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

      const source = await fetchStreamEndpoint({
        streamUrl: candidateUrl,
        apiKey: wrapperApiKey,
        sourceName: `wrapper (${cleanWrapper})`,
        signal,
        timeoutMs: 30_000,
      })

      return source
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      errors.push(`Wrapper endpoint [${candidateUrl}] failed: ${msg}`)
      debug('Candidate wrapper endpoint failed, trying next...', {
        candidateUrl,
        error: msg,
      })
    }
  }

  // 3. If standard stream endpoints failed, check if wrapper instance exposes /m3u8
  try {
    const m3u8CheckUrl = `${cleanWrapper}/m3u8?adamId=${trackId}`
    debug('Probing wrapper /m3u8 on wrapper URL...', { m3u8CheckUrl })

    const m3u8Resp = await fetch(m3u8CheckUrl, {
      headers: {
        'User-Agent': 'AlacBot/1.0',
        ...(wrapperApiKey ? { 'X-API-Key': wrapperApiKey } : {}),
      },
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(10_000)])
        : AbortSignal.timeout(10_000),
    })

    if (m3u8Resp.ok) {
      const json = (await m3u8Resp.json()) as {
        code?: number
        data?: { m3u8?: string }
      }
      if (json.data?.m3u8) {
        errors.push(
          `Wrapper is reachable and returned m3u8 playlist (${json.data.m3u8}), but direct streaming bridge endpoint is not active on this host`,
        )
      }
    }
  } catch {
    // Ignore probing errors
  }

  error('All audio stream candidates failed', {
    track_id: trackId,
    errors,
  })

  throw new Error(
    `Failed to stream audio from all sources:\n${errors.join('\n')}`,
  )
}

interface FetchEndpointParams {
  streamUrl: string
  apiKey?: string
  sourceName: string
  signal?: AbortSignal
  timeoutMs: number
}

async function fetchStreamEndpoint(
  params: FetchEndpointParams,
): Promise<AudioStreamSource> {
  const { streamUrl, apiKey, sourceName, signal, timeoutMs } = params

  const controller = new AbortController()
  const onAbort = () => controller.abort(new Error('Download was cancelled'))

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
