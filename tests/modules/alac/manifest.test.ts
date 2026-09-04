import { afterEach, beforeEach, describe, expect, it } from 'bun:test'

import { env } from '@/env.ts'
import { getMirrorEndpoint } from '@/modules/alac/manifest.ts'

describe('Mirror Manifest & Endpoint Resolution', () => {
  const originalFetch = globalThis.fetch
  const setFetch = (fn: unknown) => {
    globalThis.fetch = fn as typeof fetch
  }
  const originalMirrorUrl = env.ALAC_MIRROR_URL
  const originalApiKey = env.ALAC_API_KEY

  beforeEach(() => {
    delete (env as Record<string, unknown>).ALAC_MIRROR_URL
    delete (env as Record<string, unknown>).ALAC_API_KEY
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
    if (originalMirrorUrl !== undefined) {
      ;(env as Record<string, unknown>).ALAC_MIRROR_URL = originalMirrorUrl
    } else {
      delete (env as Record<string, unknown>).ALAC_MIRROR_URL
    }
    if (originalApiKey !== undefined) {
      ;(env as Record<string, unknown>).ALAC_API_KEY = originalApiKey
    } else {
      delete (env as Record<string, unknown>).ALAC_API_KEY
    }
  })

  it('uses configured env variables when present', async () => {
    ;(env as Record<string, unknown>).ALAC_MIRROR_URL =
      'https://custom-mirror.com/'
    ;(env as Record<string, unknown>).ALAC_API_KEY = 'secret-key-123'

    const endpoint = await getMirrorEndpoint()
    expect(endpoint.mirrorUrl).toBe('https://custom-mirror.com')
    expect(endpoint.apiKey).toBe('secret-key-123')
  })

  it('fetches manifest and verifies status successfully', async () => {
    setFetch(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('/status')) {
        return new Response(
          JSON.stringify({
            wrapper_lossless_available: true,
            wrapper_instances: [{ available: true }],
          }),
          { status: 200 },
        )
      }

      return new Response(
        JSON.stringify({
          source: { apple: 'https://mirror.alac.org/' },
          key: 'api-key-test',
        }),
        { status: 200 },
      )
    })

    const endpoint = await getMirrorEndpoint(true)
    expect(endpoint.mirrorUrl).toBe('https://mirror.alac.org')
    expect(endpoint.apiKey).toBe('api-key-test')

    const cachedEndpoint = await getMirrorEndpoint(false)
    expect(cachedEndpoint.mirrorUrl).toBe('https://mirror.alac.org')
  })

  it('throws error on manifest network timeout', async () => {
    setFetch(async () => {
      throw new Error('Network timeout')
    })

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Mirror manifest lookup timed out',
    )
  })

  it('throws error on manifest HTTP error', async () => {
    setFetch(async () => new Response('Error', { status: 500 }))

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Failed to fetch mirror manifest (HTTP 500)',
    )
  })

  it('throws error when manifest is missing apple mirror or key', async () => {
    setFetch(
      async () =>
        new Response(JSON.stringify({ empty: true }), { status: 200 }),
    )

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Mirror manifest returned empty apple endpoint or api key',
    )
  })

  it('throws error when mirror status check times out', async () => {
    setFetch(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('/status')) {
        throw new Error('Connection reset')
      }
      return new Response(
        JSON.stringify({
          mirrors: { apple: 'https://mirror-test.com' },
          api_key: 'key-123',
        }),
        { status: 200 },
      )
    })

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Mirror /status check timed out',
    )
  })

  it('throws error when mirror status returns non-200', async () => {
    setFetch(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('/status')) {
        return new Response('Offline', { status: 503 })
      }
      return new Response(
        JSON.stringify({
          mirrors: { apple: 'https://mirror-test.com' },
          api_key: 'key-123',
        }),
        { status: 200 },
      )
    })

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Mirror /status check failed (HTTP 503)',
    )
  })

  it('throws error when lossless wrapper is offline or 0 instances available', async () => {
    setFetch(async (input: RequestInfo | URL) => {
      const urlStr = String(input)
      if (urlStr.includes('/status')) {
        return new Response(
          JSON.stringify({
            wrapper_lossless_available: false,
            wrapper_instances: [],
          }),
          { status: 200 },
        )
      }
      return new Response(
        JSON.stringify({
          mirrors: { apple: 'https://mirror-test.com' },
          api_key: 'key-123',
        }),
        { status: 200 },
      )
    })

    expect(getMirrorEndpoint(true)).rejects.toThrow(
      'Lossless wrapper is currently offline on mirror',
    )
  })
})
