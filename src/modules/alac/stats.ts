import type { AlacStats } from './service.ts'

export function formatDurationMs(ms: number): string {
  if (ms <= 0) return '0ms'
  if (ms < 1000) return `${ms}ms`
  const secs = (ms / 1000).toFixed(1)
  if (ms < 60_000) return `${secs}s`
  const mins = Math.floor(ms / 60_000)
  const remSecs = Math.round((ms % 60_000) / 1000)
  return `${mins}m ${remSecs}s`
}

export function formatStatsHtml(stats: AlacStats): string {
  let topTracksSection = '<i>No completed requests yet</i>'
  if (stats.topTracks.length > 0) {
    topTracksSection = stats.topTracks
      .map(
        (t, i) =>
          `${i + 1}. <a href="https://music.apple.com/song/${t.appleTrackId}"><code>${t.appleTrackId}</code></a> — <b>${t.requestCount}</b> request${t.requestCount > 1 ? 's' : ''}`,
      )
      .join('<br/>')
  }

  return (
    `<b>📊 ALAC Bot Analytics</b><br/><br/>` +
    `<blockquote><b>📦 Storage & Caching</b><br/>` +
    `• Cached Tracks: <code>${stats.totalCachedTracks}</code><br/>` +
    `• Total Requests: <code>${stats.totalRequests}</code><br/>` +
    `• Cache Hit Ratio: <b>${stats.cacheHitRatio}%</b> (<code>${stats.cacheHits}</code> hits / <code>${stats.cacheMisses}</code> rips)<br/>` +
    `• Failed Requests: <code>${stats.totalFailedRequests}</code></blockquote><br/>` +
    `<blockquote><b>⚡ Latency Averages</b><br/>` +
    `• Cache Retrieval: <code>${formatDurationMs(stats.avgCacheDurationMs)}</code><br/>` +
    `• Mirror Rip Time: <code>${formatDurationMs(stats.avgRipDurationMs)}</code></blockquote><br/>` +
    `<blockquote><b>🔥 Top Requested Tracks</b><br/>${topTracksSection}</blockquote>`
  )
}
