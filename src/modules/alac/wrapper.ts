import {
  type AudioStreamSource,
  type ConnectStreamOptions,
  streamTransport,
} from './streaming/index.ts'

export type { AudioStreamSource, ConnectStreamOptions }

/**
 * Attempts to connect to an audio stream from the primary mirror,
 * seamlessly falling back to a configured local/remote wrapper or secondary mirror URL.
 */
export async function connectAudioStreamWithWrapper(
  options: ConnectStreamOptions,
): Promise<AudioStreamSource> {
  return streamTransport.connectAudioStream(options)
}
