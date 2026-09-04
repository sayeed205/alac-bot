import { describe, expect, it } from 'bun:test'

import { Level } from '@bcheidemann/tracing'

import { initTracing, parseLevel } from '@/utils/logger.ts'

describe('Logger Tracing', () => {
  it('parses logging levels correctly', () => {
    expect(parseLevel('trace')).toBe(Level.TRACE)
    expect(parseLevel('debug')).toBe(Level.DEBUG)
    expect(parseLevel('info')).toBe(Level.INFO)
    expect(parseLevel('warn')).toBe(Level.WARN)
    expect(parseLevel('warning')).toBe(Level.WARN)
    expect(parseLevel('error')).toBe(Level.ERROR)
    expect(parseLevel('critical')).toBe(Level.CRITICAL)
    expect(parseLevel('')).toBe(Level.INFO)
    expect(parseLevel(undefined)).toBe(Level.INFO)
    expect(parseLevel('invalid_level')).toBe(Level.INFO)
  })

  it('initializes tracing without error', () => {
    expect(() => initTracing('debug')).not.toThrow()
    expect(() => initTracing('info')).not.toThrow()
  })
})
