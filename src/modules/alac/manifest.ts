import { env } from '@/env.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'

const MANIFEST_URL_B64 =
  'aHR0cHM6Ly9naXN0LmdpdGh1YnVzZXJjb250ZW50LmNvbS9NYW5PZkluZmluaXR5L2VjNmRiNzlmMDMxZDU4NjQwYzg0YjIyNWM0Yzc4Y2FiL3Jhdw=='

interface CachedEndpoint {
  mirrorUrl: string
  apiKey: string
  expiresAt: number
}

let cached: CachedEndpoint | null = null
let lastFailure: { time: number; error: string } | null = null

export function clearMirrorCache(): void {
  cached = null
  lastFailure = null
}

export async function getMirrorEndpoint(
  forceRefresh = false,
  signal?: AbortSignal,
): Promise<{
  mirrorUrl: string
  apiKey: string
}> {
  using _ = infoSpan('mirror_endpoint').enter()

  if (env.ALAC_MIRROR_URL && env.ALAC_API_KEY) {
    const mirrorUrl = env.ALAC_MIRROR_URL.replace(/\/+$/, '')
    debug('Using configured ALAC mirror from env', { mirrorUrl })
    return {
      mirrorUrl,
      apiKey: env.ALAC_API_KEY,
    }
  }

  const now = Date.now()

  // Fast-fail if mirror failed recently and caller is not forcing a refresh
  if (!forceRefresh && lastFailure && now - lastFailure.time < 30_000) {
    debug('Using cached mirror failure (circuit breaker active)', {
      elapsed_ms: now - lastFailure.time,
      error: lastFailure.error,
    })
    throw new Error(lastFailure.error)
  }

  if (!forceRefresh && cached && cached.expiresAt > now) {
    debug('Using cached mirror endpoint', {
      mirrorUrl: cached.mirrorUrl,
      ttl_seconds: Math.round((cached.expiresAt - now) / 1000),
    })
    return { mirrorUrl: cached.mirrorUrl, apiKey: cached.apiKey }
  }

  const manifestUrl = Buffer.from(MANIFEST_URL_B64, 'base64').toString('utf-8')
  debug('Fetching latest mirror manifest...')

  const fetchStart = Date.now()
  let resp: Response
  try {
    const manifestSignal = signal
      ? AbortSignal.any([signal, AbortSignal.timeout(8_000)])
      : AbortSignal.timeout(8_000)

    resp = await fetch(manifestUrl, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
      },
      signal: manifestSignal,
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - fetchStart
    error('Failed to fetch mirror manifest due to network error/timeout', {
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    const errMsg = `Mirror manifest lookup timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  if (!resp.ok) {
    error('Mirror manifest HTTP error', { status: resp.status })
    const errMsg = `Failed to fetch mirror manifest (HTTP ${resp.status})`
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  const data = (await resp.json()) as {
    source?: { apple?: string }
    mirrors?: { apple?: string }
    key?: string
    api_key?: string
  }

  const mirror = (data.source?.apple || data.mirrors?.apple || '').replace(
    /\/+$/,
    '',
  )
  const apiKey = (data.key || data.api_key || '').trim()

  if (!mirror || !apiKey) {
    error('Mirror manifest missing endpoint or key')
    const errMsg = 'Mirror manifest returned empty apple endpoint or api key'
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  debug('Manifest fetched, verifying mirror health...', {
    mirror,
    elapsed_ms: Date.now() - fetchStart,
  })

  const statusStart = Date.now()
  let statusResp: Response
  try {
    const statusSignal = signal
      ? AbortSignal.any([signal, AbortSignal.timeout(8_000)])
      : AbortSignal.timeout(8_000)

    statusResp = await fetch(`${mirror}/status`, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0',
        'X-API-Key': apiKey,
      },
      signal: statusSignal,
    })
  } catch (err: unknown) {
    const elapsed = Date.now() - statusStart
    error('Mirror health check failed/timed out', {
      mirror,
      elapsed_ms: elapsed,
      error: err instanceof Error ? err.message : String(err),
    })
    const errMsg = `Mirror /status check timed out after ${elapsed}ms: ${err instanceof Error ? err.message : String(err)}`
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  if (!statusResp.ok) {
    error('Mirror /status HTTP error', { mirror, status: statusResp.status })
    const errMsg = `Mirror /status check failed (HTTP ${statusResp.status})`
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  const statusJson = (await statusResp.json()) as {
    wrapper_lossless_available?: boolean
    wrapper_instances?: Array<{ available?: boolean }>
  }

  const upInstances = (statusJson.wrapper_instances || []).filter(
    (i) => i.available,
  )
  if (!statusJson.wrapper_lossless_available || upInstances.length === 0) {
    error('Lossless wrapper offline on mirror', {
      lossless_available: statusJson.wrapper_lossless_available,
      up_instances: upInstances.length,
    })
    const errMsg = 'Lossless wrapper is currently offline on mirror'
    lastFailure = { time: Date.now(), error: errMsg }
    throw new Error(errMsg)
  }

  // Clear any previous failure
  lastFailure = null

  cached = {
    mirrorUrl: mirror,
    apiKey,
    expiresAt: now + 3600 * 1000, // 1 hour cache
  }

  info('Resolved active mirror endpoint', {
    mirrorUrl: mirror,
    up_instances: upInstances.length,
    elapsed_ms: Date.now() - statusStart,
  })

  return { mirrorUrl: mirror, apiKey }
}
