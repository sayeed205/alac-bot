import {
  currentSpan,
  debug,
  debugSpan,
  error,
  errorSpan,
  FmtSubscriber,
  info,
  infoSpan,
  Level,
  span,
  trace,
  traceSpan,
  warn,
  warnSpan,
} from '@bcheidemann/tracing'

export {
  currentSpan,
  debug,
  debugSpan,
  error,
  errorSpan,
  info,
  infoSpan,
  Level,
  span,
  trace,
  traceSpan,
  warn,
  warnSpan,
}

function parseLevel(raw?: string): Level {
  const normalized = (raw || '').toLowerCase().trim()
  switch (normalized) {
    case 'trace':
      return Level.TRACE
    case 'debug':
      return Level.DEBUG
    case 'info':
      return Level.INFO
    case 'warn':
    case 'warning':
      return Level.WARN
    case 'error':
      return Level.ERROR
    case 'critical':
      return Level.CRITICAL
    default:
      return Level.INFO
  }
}

let initialized = false

/**
 * Initializes the Rust-like tracing subscriber.
 * Formats events with timestamps, ANSI colors, span paths, and key-value fields.
 */
export function initTracing(levelOverride?: string): void {
  if (initialized) return

  const levelStr =
    levelOverride || process.env.LOG_LEVEL || process.env.RUST_LOG || 'info'
  const level = parseLevel(levelStr)

  FmtSubscriber.setGlobalDefault({
    level,
    color: true,
    abbreviateLongFieldValues: 40,
  })

  initialized = true
}
