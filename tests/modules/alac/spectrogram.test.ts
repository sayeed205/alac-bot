import { afterAll, beforeAll, describe, expect, it } from 'bun:test'
import { existsSync, unlinkSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { generateSpectrogram, probeAudio } from '@/modules/alac/spectrogram.ts'

describe('Audio Spectrogram & Metadata Probe', () => {
  const tmpDir = os.tmpdir()
  const testWav = path.join(tmpDir, `test_audio_${Date.now()}.wav`)
  const testPng = path.join(tmpDir, `test_spec_${Date.now()}.png`)

  beforeAll(async () => {
    // Generate a 1-second 440Hz + 8000Hz test audio file using ffmpeg
    const proc = Bun.spawn([
      'ffmpeg',
      '-v',
      'error',
      '-f',
      'lavfi',
      '-i',
      'sine=frequency=440:duration=1',
      '-metadata',
      'title=Test Sinewave',
      '-metadata',
      'artist=Test Lab',
      '-metadata',
      'album=Acoustics',
      '-y',
      testWav,
    ])
    await proc.exited
  })

  afterAll(() => {
    if (existsSync(testWav)) {
      try {
        unlinkSync(testWav)
      } catch {}
    }
    if (existsSync(testPng)) {
      try {
        unlinkSync(testPng)
      } catch {}
    }
  })

  it('probes audio metadata and tags using ffprobe', async () => {
    const probe = await probeAudio(testWav)

    expect(probe).toBeDefined()
    expect(probe.codec).toBe('pcm_s16le')
    expect(probe.sampleRate).toBe(44100)
    expect(probe.channels).toBe(1)
    expect(probe.duration).toBeGreaterThanOrEqual(0.9)
    expect(probe.title).toBe('Test Sinewave')
    expect(probe.artist).toBe('Test Lab')
    expect(probe.album).toBe('Acoustics')
  })

  it('generates a valid spectrogram PNG using SoX', async () => {
    await generateSpectrogram(testWav, testPng, {
      title: 'Test Lab - Test Sinewave',
      comment: 'WAV 16-bit • 44100 Hz',
      width: 800,
      height: 400,
    })

    expect(existsSync(testPng)).toBe(true)
    const stat = await Bun.file(testPng).stat()
    expect(stat.size).toBeGreaterThan(1000)
  })

  it('throws error when probing an invalid or non-existent file', async () => {
    expect(probeAudio('/tmp/non_existent_file_xyz.wav')).rejects.toThrow()
  })
})
