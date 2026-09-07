import { describe, expect, test } from 'bun:test'

import { connectAudioStreamWithWrapper } from '@/modules/alac/wrapper.ts'

describe('connectAudioStreamWithWrapper', () => {
  test('connects to primary mirror when it responds successfully', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('primary.mirror/api/stream/12345')) {
        return new Response('mock-audio-bytes', {
          status: 200,
          headers: {
            'content-type': 'audio/mp4',
            'x-codec': 'alac',
            'x-bitdepth': '24',
            'x-samplerate': '96000',
          },
        })
      }
      return new Response('Not Found', { status: 404 })
    }) as unknown as typeof fetch

    try {
      const source = await connectAudioStreamWithWrapper({
        trackId: '12345',
        primaryMirror: {
          mirrorUrl: 'https://primary.mirror',
          apiKey: 'test-key',
        },
        wrapperUrl: 'http://127.0.0.1:12340',
      })

      expect(source.sourceName).toContain('primary mirror')
      expect(source.codec).toBe('alac')
      expect(source.bitDepth).toBe(24)
      expect(source.sampleRate).toBe(96000)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  test('falls back to wrapperUrl when primary mirror fails with 502', async () => {
    const originalFetch = globalThis.fetch
    let wrapperHit = false

    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('primary.mirror/api/stream/12345')) {
        return new Response('Bad Gateway', { status: 502 })
      }
      if (urlStr.includes('127.0.0.1:12340/api/stream/12345')) {
        wrapperHit = true
        return new Response('wrapper-audio-bytes', {
          status: 200,
          headers: {
            'content-type': 'audio/mp4',
            'x-codec': 'alac',
            'x-bitdepth': '16',
            'x-samplerate': '44100',
          },
        })
      }
      return new Response('Not Found', { status: 404 })
    }) as unknown as typeof fetch

    try {
      const statusUpdates: string[] = []
      const source = await connectAudioStreamWithWrapper({
        trackId: '12345',
        primaryMirror: {
          mirrorUrl: 'https://primary.mirror',
          apiKey: 'test-key',
        },
        wrapperUrl: 'http://127.0.0.1:12340',
        onProgress: (s: string) => statusUpdates.push(s),
      })

      expect(wrapperHit).toBe(true)
      expect(source.sourceName).toContain('wrapper')
      expect(source.codec).toBe('alac')
      expect(source.bitDepth).toBe(16)
      expect(source.sampleRate).toBe(44100)
      expect(statusUpdates.some((s) => s.includes('wrapper'))).toBe(true)
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  test('falls back directly when primary mirror is null (e.g. manifest down)', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = (async (url: string | URL | Request) => {
      const urlStr = url.toString()
      if (urlStr.includes('custom-wrapper.com/stream/99999')) {
        return new Response('wrapper-stream', {
          status: 200,
          headers: {
            'x-codec': 'alac',
          },
        })
      }
      return new Response('Not Found', { status: 404 })
    }) as unknown as typeof fetch

    try {
      const source = await connectAudioStreamWithWrapper({
        trackId: '99999',
        primaryMirror: null,
        wrapperUrl: 'https://custom-wrapper.com',
      })

      expect(source.sourceName).toContain(
        'wrapper (https://custom-wrapper.com)',
      )
      expect(source.codec).toBe('alac')
    } finally {
      globalThis.fetch = originalFetch
    }
  })

  test('throws comprehensive error when all sources fail', async () => {
    const originalFetch = globalThis.fetch
    globalThis.fetch = (async () => {
      return new Response('Service Unavailable', { status: 503 })
    }) as unknown as typeof fetch

    try {
      expect(
        connectAudioStreamWithWrapper({
          trackId: '12345',
          primaryMirror: {
            mirrorUrl: 'https://primary.mirror',
            apiKey: 'test-key',
          },
          wrapperUrl: 'http://127.0.0.1:12340',
        }),
      ).rejects.toThrow('Failed to stream audio from all sources')
    } finally {
      globalThis.fetch = originalFetch
    }
  })
})
