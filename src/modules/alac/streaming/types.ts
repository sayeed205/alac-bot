import type { IMirrorPolicy } from './mirror-policy.ts'

export interface AudioStreamSource {
  streamResp: Response
  sourceName: string
  codec: string
  bitDepth: number
  sampleRate: number
}

export interface ConnectStreamOptions {
  trackId: string
  primaryMirror: { mirrorUrl: string; apiKey: string } | null
  wrapperUrl?: string
  wrapperApiKey?: string
  signal?: AbortSignal
  onProgress?: (status: string) => void
  mirrorPolicy?: IMirrorPolicy
}
