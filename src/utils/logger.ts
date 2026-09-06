import {
  createSubscriberContext,
  currentSpan,
  debug,
  debugSpan,
  type Event,
  error,
  errorSpan,
  FmtSubscriber,
  type FmtSubscriberOptions,
  info,
  infoSpan,
  Level,
  type SpanAttributes,
  setDefaultGlobalSubscriber,
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

export function parseLevel(raw?: string): Level {
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

/**
 * Custom FmtSubscriber that provides safe concurrent span tracking and deduplication.
 * Default ManagedSubscriber from @bcheidemann/tracing links parent spans linearly on a
 * shared instance variable, which corrupts spans when multiple async tasks enter concurrently
 * (e.g., repeating prefixes like alac:alac:alac:alac or prematurely wiping spans on exit).
 */
export class RobustFmtSubscriber extends FmtSubscriber {
  private readonly _activeSpans: { id: symbol; attributes: SpanAttributes }[] =
    []
  private readonly _pending = new Map<
    symbol,
    { id: symbol; attributes: SpanAttributes }
  >()

  constructor(options: FmtSubscriberOptions = {}) {
    super(options)
  }

  override newSpan(attributes: SpanAttributes): symbol {
    const id = Symbol()
    this._pending.set(id, { id, attributes })
    return id
  }

  override currentSpan(): symbol | undefined {
    return this._activeSpans.length > 0
      ? this._activeSpans[this._activeSpans.length - 1].id
      : undefined
  }

  override enter(spanId: symbol): void {
    const pending = this._pending.get(spanId)
    if (!pending) return
    this._activeSpans.push({ id: spanId, attributes: pending.attributes })
  }

  override exit(spanId: symbol): void {
    this._pending.delete(spanId)
    for (let i = this._activeSpans.length - 1; i >= 0; i--) {
      if (this._activeSpans[i].id === spanId) {
        this._activeSpans.splice(i, 1)
        break
      }
    }
  }

  override record(spanId: symbol, key: string, value: unknown): void {
    const pending = this._pending.get(spanId)
    if (pending) {
      pending.attributes.fields = pending.attributes.fields || {}
      pending.attributes.fields[key] = value
    }
    const active = this._activeSpans.find((s) => s.id === spanId)
    if (active) {
      active.attributes.fields = active.attributes.fields || {}
      active.attributes.fields[key] = value
    }
  }

  override event(event: Event): void {
    const deduped: SpanAttributes[] = []
    const seenMessages = new Set<string>()

    // Traverse from most recent active span to oldest, deduplicating repeated span names
    for (let i = this._activeSpans.length - 1; i >= 0; i--) {
      const activeSpan = this._activeSpans[i]
      if (!seenMessages.has(activeSpan.attributes.message)) {
        seenMessages.add(activeSpan.attributes.message)
        deduped.push(activeSpan.attributes)
      }
    }

    // Pass deduplicated spans to FmtSubscriber for display
    this.onEvent(event, deduped)
  }

  override clone(): RobustFmtSubscriber {
    return this
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

  const subscriber = new RobustFmtSubscriber({
    level,
    color: true,
    abbreviateLongFieldValues: 40,
  })

  setDefaultGlobalSubscriber(createSubscriberContext(subscriber))

  initialized = true
}
