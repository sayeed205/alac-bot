import { existsSync } from 'node:fs'

import { debug, error, infoSpan } from '@/utils/logger.ts'

export interface AudioProbeResult {
  title?: string
  artist?: string
  album?: string
  codec: string
  sampleRate: number
  bitDepth?: number
  channels: number
  channelLayout?: string
  bitRate?: number
  duration: number
}

export interface SpectrogramOptions {
  title?: string
  comment?: string
  width?: number
  height?: number
  dynamicRange?: number
  duration?: number
}

interface FfprobeStream {
  codec_name?: string
  codec_type?: string
  sample_rate?: string
  channels?: number
  channel_layout?: string
  bits_per_sample?: number
  bits_per_raw_sample?: string
  duration?: string
  bit_rate?: string
  tags?: Record<string, string>
}

interface FfprobeFormat {
  duration?: string
  bit_rate?: string
  tags?: Record<string, string>
}

interface FfprobeOutput {
  streams?: FfprobeStream[]
  format?: FfprobeFormat
}

/**
 * Probes audio metadata from a media file using ffprobe.
 */
export async function probeAudio(filePath: string): Promise<AudioProbeResult> {
  using _ = infoSpan('probe_audio').enter()

  const proc = Bun.spawn(
    [
      'ffprobe',
      '-v',
      'quiet',
      '-print_format',
      'json',
      '-show_format',
      '-show_streams',
      filePath,
    ],
    { stdout: 'pipe', stderr: 'pipe' },
  )

  const exitCode = await proc.exited
  if (exitCode !== 0) {
    throw new Error(`ffprobe exited with code ${exitCode}`)
  }

  const rawJson = await new Response(proc.stdout).text()
  const data: FfprobeOutput = JSON.parse(rawJson)

  const audioStream = data.streams?.find((s) => s.codec_type === 'audio')
  if (!audioStream) {
    throw new Error('No audio stream detected in media file')
  }

  const tags = {
    ...data.format?.tags,
    ...audioStream.tags,
  }

  const findTag = (keys: string[]): string | undefined => {
    for (const key of keys) {
      for (const [k, v] of Object.entries(tags)) {
        if (k.toLowerCase() === key.toLowerCase() && v) {
          return String(v).trim()
        }
      }
    }
    return undefined
  }

  const sampleRate = Number.parseInt(audioStream.sample_rate || '0', 10)
  const channels = audioStream.channels || 2
  const durationSec = Number.parseFloat(
    audioStream.duration || data.format?.duration || '0',
  )

  const rawBits =
    Number.parseInt(audioStream.bits_per_raw_sample || '0', 10) ||
    audioStream.bits_per_sample ||
    0
  const bitDepth = rawBits > 0 ? rawBits : undefined

  const bitRate =
    Number.parseInt(audioStream.bit_rate || data.format?.bit_rate || '0', 10) ||
    undefined

  return {
    title: findTag(['title', 'song', 'track']),
    artist: findTag(['artist', 'album_artist', 'performer']),
    album: findTag(['album']),
    codec: audioStream.codec_name || 'unknown',
    sampleRate: sampleRate > 0 ? sampleRate : 44100,
    bitDepth,
    channels,
    channelLayout: audioStream.channel_layout,
    bitRate,
    duration: durationSec > 0 ? durationSec : 0,
  }
}

/**
 * Generates an audio spectrogram using SoX (with fallback to FFmpeg pipeline and showspectrumpic).
 */
export async function generateSpectrogram(
  inputPath: string,
  outputPath: string,
  options: SpectrogramOptions = {},
): Promise<void> {
  using _ = infoSpan('generate_spectrogram').enter()

  const width = options.width ?? 1200
  const height = options.height ?? 551
  const dynamicRange = options.dynamicRange ?? 120

  let duration = options.duration
  if (!duration || duration <= 0) {
    try {
      const probe = await probeAudio(inputPath)
      duration = probe.duration
    } catch {
      // Ignore probe errors and proceed
    }
  }

  // 1. Attempt direct SoX spectrogram
  try {
    const soxArgs = [
      'sox',
      inputPath,
      '-n',
      'spectrogram',
      '-x',
      String(width),
      '-y',
      String(height),
      '-z',
      String(dynamicRange),
    ]

    if (duration && duration > 0) {
      soxArgs.push('-d', duration.toFixed(2))
    }

    if (options.title) {
      soxArgs.push('-t', options.title)
    }
    if (options.comment) {
      soxArgs.push('-c', options.comment)
    }
    soxArgs.push('-o', outputPath)

    const proc = Bun.spawn(soxArgs, { stdout: 'ignore', stderr: 'pipe' })
    const exitCode = await proc.exited

    if (exitCode === 0 && existsSync(outputPath)) {
      debug('SoX direct spectrogram generated successfully', { outputPath })
      return
    }
  } catch (err) {
    debug('Direct SoX spectrogram failed, trying piped ffmpeg', {
      error: String(err),
    })
  }

  // 2. Attempt FFmpeg WAV decode piped into SoX
  try {
    const ffmpegProc = Bun.spawn(
      ['ffmpeg', '-v', 'error', '-i', inputPath, '-f', 'wav', '-'],
      { stdout: 'pipe', stderr: 'pipe' },
    )

    const soxPipeArgs = [
      'sox',
      '-t',
      'wav',
      '-',
      '-n',
      'spectrogram',
      '-x',
      String(width),
      '-y',
      String(height),
      '-z',
      String(dynamicRange),
    ]

    if (duration && duration > 0) {
      soxPipeArgs.push('-d', duration.toFixed(2))
    }

    if (options.title) {
      soxPipeArgs.push('-t', options.title)
    }
    if (options.comment) {
      soxPipeArgs.push('-c', options.comment)
    }
    soxPipeArgs.push('-o', outputPath)

    const soxProc = Bun.spawn(soxPipeArgs, {
      stdin: ffmpegProc.stdout,
      stdout: 'ignore',
      stderr: 'pipe',
    })

    const exitCode = await soxProc.exited
    if (exitCode === 0 && existsSync(outputPath)) {
      debug('Piped SoX spectrogram generated successfully', { outputPath })
      return
    }
  } catch (err) {
    debug(
      'Piped SoX spectrogram failed, falling back to ffmpeg showspectrumpic',
      {
        error: String(err),
      },
    )
  }

  // 3. Fallback to FFmpeg showspectrumpic filter
  const ffmpegFilterProc = Bun.spawn(
    [
      'ffmpeg',
      '-v',
      'error',
      '-i',
      inputPath,
      '-lavfi',
      `showspectrumpic=s=${width}x${height}:mode=combined:color=intensity:scale=log:fscale=lin:legend=true`,
      '-y',
      outputPath,
    ],
    { stdout: 'ignore', stderr: 'pipe' },
  )

  const exitCode = await ffmpegFilterProc.exited
  if (exitCode !== 0 || !existsSync(outputPath)) {
    error('All spectrogram generation methods failed', { inputPath, exitCode })
    throw new Error(`Failed to generate spectrogram (exit code: ${exitCode})`)
  }

  debug('FFmpeg showspectrumpic generated successfully', { outputPath })
}
