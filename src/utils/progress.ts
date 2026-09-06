/**
 * Renders a clean, aesthetic Unicode block progress bar without any emojis.
 * Example output: "[██████░░░░░░] 50%"
 */
export function renderProgressBar(
  current: number,
  total: number,
  length = 12,
): string {
  if (total <= 0 || Number.isNaN(total) || Number.isNaN(current)) {
    return `[${'░'.repeat(length)}] 0%`
  }

  const fraction = Math.min(Math.max(current / total, 0), 1)
  const filledCount = Math.round(fraction * length)
  const emptyCount = length - filledCount
  const bar = '█'.repeat(filledCount) + '░'.repeat(emptyCount)
  const percent = Math.round(fraction * 100)

  return `[${bar}] ${percent}%`
}

/**
 * Formats MB progress as "x.x/y.y MB" without progress bar.
 * Example: "14.5/29.0 MB"
 */
export function formatMbProgress(
  currentBytes: number,
  totalBytes: number,
): string {
  const currentMb = (currentBytes / (1024 * 1024)).toFixed(1)
  if (totalBytes > 0) {
    const totalMb = (totalBytes / (1024 * 1024)).toFixed(1)
    return `${currentMb}/${totalMb} MB`
  }
  return `${currentMb} MB`
}

/**
 * Formats a byte progress string with progress bar.
 * Example:
 * "[██████░░░░░░] 50% (14.5 / 29.0 MB)"
 */
export function formatByteProgress(
  currentBytes: number,
  totalBytes: number,
  barLength = 12,
): string {
  const bar = renderProgressBar(currentBytes, totalBytes, barLength)
  const currentMb = (currentBytes / (1024 * 1024)).toFixed(1)

  if (totalBytes > 0) {
    const totalMb = (totalBytes / (1024 * 1024)).toFixed(1)
    return `${bar} (${currentMb}/${totalMb} MB)`
  }

  return `${currentMb} MB`
}

/**
 * Formats a byte number into human-readable representation (e.g. "12.4 MB").
 */
export function formatBytes(bytes: number): string {
  if (bytes <= 0 || Number.isNaN(bytes)) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  const unitIndex = Math.min(Math.max(i, 0), units.length - 1)
  const val = (bytes / 1024 ** unitIndex).toFixed(unitIndex === 0 ? 0 : 2)
  return `${val} ${units[unitIndex]}`
}
