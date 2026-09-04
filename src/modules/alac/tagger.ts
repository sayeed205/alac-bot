import { unlink } from 'node:fs/promises'

import type { AppleTrackMetadata } from './types.ts'

export function sanitizeFilename(name: string): string {
  return name.replace(/[<>:"/\\|?*]/g, '_').trim() || 'track'
}

export function buildTrackFilename(meta: AppleTrackMetadata): string {
  const n = String(meta.trackNumber || 1).padStart(2, '0')
  const explicit = meta.explicit ? ' [E]' : ''
  return `${sanitizeFilename(`${n}. ${meta.title} - ${meta.artist}${explicit} [ALAC]`)}.m4a`
}

export async function tagM4aFile(params: {
  rawAudioPath: string
  outputPath: string
  meta: AppleTrackMetadata
  coverBuffer?: Uint8Array | null
  lyrics?: string | null
}): Promise<string> {
  const { rawAudioPath, outputPath, meta, coverBuffer, lyrics } = params

  let tempCoverPath: string | null = null
  if (coverBuffer && coverBuffer.byteLength > 0) {
    tempCoverPath = `${outputPath}.cover.jpg`
    await Bun.write(tempCoverPath, coverBuffer)
  }

  const args: string[] = ['ffmpeg', '-y', '-i', rawAudioPath]

  if (tempCoverPath) {
    args.push('-i', tempCoverPath)
    args.push('-map', '0:a', '-map', '1')
    args.push('-c', 'copy')
    args.push('-disposition:v:0', 'attached_pic')
  } else {
    args.push('-c', 'copy')
  }

  // Metadata tags
  if (meta.title) args.push('-metadata', `title=${meta.title}`)
  if (meta.artist) args.push('-metadata', `artist=${meta.artist}`)
  if (meta.album) args.push('-metadata', `album=${meta.album}`)
  if (meta.albumArtist)
    args.push('-metadata', `album_artist=${meta.albumArtist}`)
  if (meta.releaseDate) args.push('-metadata', `date=${meta.releaseDate}`)
  if (meta.genre) args.push('-metadata', `genre=${meta.genre}`)
  if (meta.composer) args.push('-metadata', `composer=${meta.composer}`)
  if (meta.trackNumber) {
    args.push(
      '-metadata',
      `track=${meta.trackNumber}${meta.trackCount ? `/${meta.trackCount}` : ''}`,
    )
  }
  if (meta.discNumber) {
    args.push(
      '-metadata',
      `disc=${meta.discNumber}${meta.discCount ? `/${meta.discCount}` : ''}`,
    )
  }
  if (lyrics) {
    args.push('-metadata', `lyrics=${lyrics}`)
  }

  args.push(outputPath)

  try {
    const proc = Bun.spawn(args, {
      stdout: 'pipe',
      stderr: 'pipe',
    })

    const exitCode = await proc.exited
    if (exitCode !== 0) {
      const errText = await new Response(proc.stderr).text()
      throw new Error(
        `FFmpeg tagging failed (exit code ${exitCode}): ${errText.slice(-200)}`,
      )
    }

    return outputPath
  } finally {
    if (tempCoverPath) {
      await unlink(tempCoverPath).catch(() => {})
    }
  }
}
