import { describe, expect, it } from 'bun:test'

import { type Event, Level, type SpanAttributes } from '@bcheidemann/tracing'

import { initTracing, parseLevel, RobustFmtSubscriber } from '@/utils/logger.ts'

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

  it('deduplicates repeated concurrent spans in RobustFmtSubscriber', () => {
    let capturedSpans: SpanAttributes[] = []

    class TestSubscriber extends RobustFmtSubscriber {
      protected override onEvent(_event: Event, spans: SpanAttributes[]) {
        capturedSpans = spans
      }
    }

    const subscriber = new TestSubscriber({ level: Level.INFO })
    const span1 = subscriber.newSpan({
      isSpan: true,
      level: Level.INFO,
      message: 'alac',
    })
    const span2 = subscriber.newSpan({
      isSpan: true,
      level: Level.INFO,
      message: 'alac',
    })
    const span3 = subscriber.newSpan({
      isSpan: true,
      level: Level.INFO,
      message: 'itunes_track',
      fields: { track_id: 123 },
    })

    subscriber.enter(span1)
    subscriber.enter(span2)
    subscriber.enter(span3)

    subscriber.event({
      isEvent: true,
      level: Level.INFO,
      message: 'test message',
    })

    // Should only contain 'itunes_track' and 'alac' (deduplicated), not two 'alac's
    expect(capturedSpans.length).toBe(2)
    expect(capturedSpans[0].message).toBe('itunes_track')
    expect(capturedSpans[1].message).toBe('alac')

    // Exit span3 and span1
    subscriber.exit(span3)
    subscriber.exit(span1)

    subscriber.event({
      isEvent: true,
      level: Level.INFO,
      message: 'after exit',
    })

    // span2 is still active, so 'alac' remains
    expect(capturedSpans.length).toBe(1)
    expect(capturedSpans[0].message).toBe('alac')

    subscriber.exit(span2)

    subscriber.event({
      isEvent: true,
      level: Level.INFO,
      message: 'all exited',
    })

    expect(capturedSpans.length).toBe(0)
  })
})
