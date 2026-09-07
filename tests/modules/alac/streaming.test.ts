import { afterEach, describe, expect, it } from 'bun:test'

import {
  MirrorPolicyManager,
  StreamTransport,
} from '@/modules/alac/streaming/index.ts'

describe('Streaming Module (MirrorPolicyManager & StreamTransport)', () => {
  const originalFetch = globalThis.fetch

  const setFetch = (fn: unknown) => {
    globalThis.fetch = fn as typeof fetch
  }

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  describe('MirrorPolicyManager', () => {
    it('opens circuit when failure is recorded and prevents redundant network calls', async () => {
      const policy = new MirrorPolicyManager(5000, 60000, 1000)
      expect(policy.isCircuitOpen()).toBe(false)

      policy.recordFailure('Mirror 500 error')
      expect(policy.isCircuitOpen()).toBe(true)

      // getEndpoint should immediately reject because circuit is open
      expect(policy.getEndpoint(false)).rejects.toThrow('Mirror 500 error')

      // recordSuccess clears failure
      policy.recordSuccess()
      expect(policy.isCircuitOpen()).toBe(false)
    })
  })

  describe('StreamTransport', () => {
    it('fetches stream endpoint and parses headers accurately', async () => {
      setFetch(async () => {
        return new Response('fake-audio-payload', {
          status: 200,
          headers: {
            'x-codec': 'alac',
            'x-bitdepth': '24',
            'x-samplerate': '192000',
          },
        })
      })

      const transport = new StreamTransport(5000)
      const res = await transport.fetchEndpoint({
        streamUrl: 'https://example.com/api/stream/12345',
        sourceName: 'test-source',
        timeoutMs: 5000,
      })

      expect(res.sourceName).toBe('test-source')
      expect(res.codec).toBe('alac')
      expect(res.bitDepth).toBe(24)
      expect(res.sampleRate).toBe(192000)
    })

    it('trips circuit breaker on mirror failure and falls back to wrapper', async () => {
      const policy = new MirrorPolicyManager(5000, 60000, 1000)
      expect(policy.isCircuitOpen()).toBe(false)

      setFetch(async (url: string | URL | Request) => {
        const urlStr = String(url)
        if (urlStr.includes('primary.mirror.com')) {
          return new Response('Primary Mirror Internal Error', { status: 502 })
        }
        if (urlStr.includes('fallback-wrapper.com')) {
          return new Response('fake-wrapper-audio', {
            status: 200,
            headers: {
              'x-codec': 'alac',
              'x-bitdepth': '16',
              'x-samplerate': '44100',
            },
          })
        }
        return new Response('Not Found', { status: 404 })
      })

      const transport = new StreamTransport(5000)
      const res = await transport.connectAudioStream({
        trackId: '999',
        primaryMirror: {
          mirrorUrl: 'https://primary.mirror.com',
          apiKey: 'key123',
        },
        wrapperUrl: 'https://fallback-wrapper.com',
        mirrorPolicy: policy,
      })

      expect(res.sourceName).toContain('wrapper')
      expect(res.codec).toBe('alac')
      // Primary mirror failure should have tripped the circuit breaker
      expect(policy.isCircuitOpen()).toBe(true)
    })
  })
})
