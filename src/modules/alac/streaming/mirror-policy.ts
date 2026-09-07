import { env } from '@/env.ts'
import { debug, error, info, infoSpan } from '@/utils/logger.ts'

const MANIFEST_URL_B64 =
  'aHR0cHM6Ly9naXN0LmdpdGh1YnVzZXJjb250ZW50LmNvbS9NYW5PZkluZmluaXR5L2VjNmRiNzlmMDMxZDU4NjQwYzg0YjIyNWM0Yzc4Y2FiL3Jhdw=='

export interface MirrorEndpoint {
  mirrorUrl: string
  apiKey: string
}

export interface CachedEndpoint extends MirrorEndpoint {
  expiresAt: number
}

export interface IMirrorPolicy {
  getEndpoint(
    forceRefresh?: boolean,
    signal?: AbortSignal,
  ): Promise<MirrorEndpoint>
  clearCache(): void
  isCircuitOpen(): boolean
  recordFailure(errorMsg: string): void
  recordSuccess(): void
}

export class MirrorPolicyManager implements IMirrorPolicy {
  private cached: CachedEndpoint | null = null
  private lastFailure: { time: number; error: string } | null = null
  private readonly failureCooldownMs: number
  private readonly cacheTtlMs: number
  private readonly healthTimeoutMs: number

  constructor(
    failureCooldownMs = 30_000,
    cacheTtlMs = 60 * 60 * 1000, // 1 hour
    healthTimeoutMs = 8_000,
  ) {
    this.failureCooldownMs = failureCooldownMs
    this.cacheTtlMs = cacheTtlMs
    this.healthTimeoutMs = healthTimeoutMs
  }

  public clearCache(): void {
    this.cached = null
    this.lastFailure = null
  }

  public isCircuitOpen(): boolean {
    if (!this.lastFailure) return false
    return Date.now() - this.lastFailure.time < this.failureCooldownMs
  }

  public recordFailure(errorMsg: string): void {
    this.lastFailure = { time: Date.now(), error: errorMsg }
  }

  public recordSuccess(): void {
    this.lastFailure = null
  }

  public async getEndpoint(
    forceRefresh = false,
    signal?: AbortSignal,
  ): Promise<MirrorEndpoint> {
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

    // Fast-fail if mirror circuit is open and caller is not forcing a refresh
    if (!forceRefresh && this.isCircuitOpen() && this.lastFailure) {
      debug('Using cached mirror failure (circuit breaker active)', {
        elapsed_ms: now - this.lastFailure.time,
        error: this.lastFailure.error,
      })
      throw new Error(this.lastFailure.error)
    }

    if (!forceRefresh && this.cached && this.cached.expiresAt > now) {
      debug('Using cached mirror endpoint', {
        mirrorUrl: this.cached.mirrorUrl,
        ttl_seconds: Math.round((this.cached.expiresAt - now) / 1000),
      })
      return { mirrorUrl: this.cached.mirrorUrl, apiKey: this.cached.apiKey }
    }

    const manifestUrl = Buffer.from(MANIFEST_URL_B64, 'base64').toString(
      'utf-8',
    )
    debug('Fetching latest mirror manifest...')

    const fetchStart = Date.now()
    let resp: Response
    try {
      const manifestSignal = signal
        ? AbortSignal.any([signal, AbortSignal.timeout(this.healthTimeoutMs)])
        : AbortSignal.timeout(this.healthTimeoutMs)

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
      this.recordFailure(errMsg)
      throw new Error(errMsg)
    }

    if (!resp.ok) {
      error('Mirror manifest HTTP error', { status: resp.status })
      const errMsg = `Failed to fetch mirror manifest (HTTP ${resp.status})`
      this.recordFailure(errMsg)
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
      this.recordFailure(errMsg)
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
        ? AbortSignal.any([signal, AbortSignal.timeout(this.healthTimeoutMs)])
        : AbortSignal.timeout(this.healthTimeoutMs)

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
      this.recordFailure(errMsg)
      throw new Error(errMsg)
    }

    if (!statusResp.ok) {
      error('Mirror /status HTTP error', { mirror, status: statusResp.status })
      const errMsg = `Mirror /status check failed (HTTP ${statusResp.status})`
      this.recordFailure(errMsg)
      throw new Error(errMsg)
    }

    const statusData = (await statusResp.json().catch(() => ({}))) as {
      wrapper_lossless_available?: boolean
      wrapper_instances?: unknown[]
    }

    if (
      statusData.wrapper_lossless_available === false ||
      (Array.isArray(statusData.wrapper_instances) &&
        statusData.wrapper_instances.length === 0)
    ) {
      error('Lossless wrapper is offline on mirror', { mirror, statusData })
      const errMsg = 'Lossless wrapper is currently offline on mirror'
      this.recordFailure(errMsg)
      throw new Error(errMsg)
    }

    info('Mirror verified successfully', {
      mirror,
      elapsed_ms: Date.now() - statusStart,
    })

    this.cached = {
      mirrorUrl: mirror,
      apiKey,
      expiresAt: Date.now() + this.cacheTtlMs,
    }
    this.recordSuccess()

    return { mirrorUrl: mirror, apiKey }
  }
}

export const mirrorPolicyManager = new MirrorPolicyManager()
