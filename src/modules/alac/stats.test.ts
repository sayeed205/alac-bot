import { describe, expect, it } from 'bun:test'

import type { AlacStats } from './service.ts'
import { formatDurationMs, formatStatsHtml } from './stats.ts'

describe('Stats Formatter', () => {
  it('formats duration in milliseconds, seconds, and minutes correctly', () => {
    expect(formatDurationMs(0)).toBe('0ms')
    expect(formatDurationMs(450)).toBe('450ms')
    expect(formatDurationMs(1500)).toBe('1.5s')
    expect(formatDurationMs(65000)).toBe('1m 5s')
  })

  it('renders stats html with top tracks', () => {
    const mockStats: AlacStats = {
      totalCachedTracks: 12,
      totalRequests: 25,
      cacheHits: 15,
      cacheMisses: 10,
      cacheHitRatio: 60,
      avgRipDurationMs: 18500,
      avgCacheDurationMs: 320,
      totalFailedRequests: 1,
      topTracks: [
        { appleTrackId: '1559523359', requestCount: 8 },
        { appleTrackId: '1193701400', requestCount: 3 },
      ],
    }

    const html = formatStatsHtml(mockStats)
    expect(html).toContain('<b>📊 ALAC Bot Analytics</b>')
    expect(html).toContain('Cached Tracks: <code>12</code>')
    expect(html).toContain('Total Requests: <code>25</code>')
    expect(html).toContain('Cache Hit Ratio: <b>60%</b>')
    expect(html).toContain('1559523359')
    expect(html).toContain('<b>8</b> requests')
  })

  it('renders fallback when no top tracks exist', () => {
    const emptyStats: AlacStats = {
      totalCachedTracks: 0,
      totalRequests: 0,
      cacheHits: 0,
      cacheMisses: 0,
      cacheHitRatio: 0,
      avgRipDurationMs: 0,
      avgCacheDurationMs: 0,
      totalFailedRequests: 0,
      topTracks: [],
    }

    const html = formatStatsHtml(emptyStats)
    expect(html).toContain('<i>No completed requests yet</i>')
  })
})
