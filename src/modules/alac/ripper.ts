import { existsSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

export interface TrackRipResult {
  filePath: string
  title: string
  artist: string
  album?: string
  duration?: number
}

export interface ITrackRipper {
  rip(
    trackId: string,
    onProgress?: (status: string) => void,
  ): Promise<TrackRipResult>
}

export class FakeTrackRipper implements ITrackRipper {
  constructor(private mockResult?: Partial<TrackRipResult>) {}

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
  ): Promise<TrackRipResult> {
    onProgress?.('Downloading metadata...')
    await new Promise((r) => setTimeout(r, 10))
    onProgress?.('Fetching ALAC stream...')
    await new Promise((r) => setTimeout(r, 10))

    return {
      filePath: this.mockResult?.filePath || `/tmp/mock_${trackId}.m4a`,
      title: this.mockResult?.title || `Mock Song ${trackId}`,
      artist: this.mockResult?.artist || 'Mock Artist',
      album: this.mockResult?.album || 'Mock Album',
      duration: this.mockResult?.duration || 210,
    }
  }
}

function findLatestM4aFile(dir: string): string | null {
  if (!existsSync(dir)) return null

  let latestFile: string | null = null
  let latestMtime = 0

  function scan(currentDir: string) {
    const entries = readdirSync(currentDir, { withFileTypes: true })
    for (const entry of entries) {
      const fullPath = join(currentDir, entry.name)
      if (entry.isDirectory()) {
        scan(fullPath)
      } else if (entry.isFile() && entry.name.endsWith('.m4a')) {
        const stats = statSync(fullPath)
        if (stats.mtimeMs > latestMtime) {
          latestMtime = stats.mtimeMs
          latestFile = fullPath
        }
      }
    }
  }

  scan(dir)
  return latestFile
}

export class AppleBruhRipper implements ITrackRipper {
  private pythonPath: string
  private scriptPath: string
  private outputDir: string

  constructor(options?: {
    pythonPath?: string
    scriptPath?: string
    outputDir?: string
  }) {
    this.pythonPath =
      options?.pythonPath ||
      process.env.ALAC_PYTHON_PATH ||
      '/home/hitarashi/sayeed/github/applebruh/.venv/bin/python3'
    this.scriptPath =
      options?.scriptPath ||
      process.env.ALAC_SCRIPT_PATH ||
      '/home/hitarashi/sayeed/github/applebruh/alac.py'
    this.outputDir =
      options?.outputDir ||
      process.env.ALAC_OUTPUT_DIR ||
      join(process.cwd(), 'bot-data', 'downloads')
  }

  async rip(
    trackId: string,
    onProgress?: (status: string) => void,
  ): Promise<TrackRipResult> {
    onProgress?.('Connecting to Apple Music server...')

    const proc = Bun.spawn(
      [
        this.pythonPath,
        this.scriptPath,
        '--track-id',
        trackId,
        '--out',
        this.outputDir,
      ],
      {
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )

    let artist = 'Unknown Artist'
    let title = `Track ${trackId}`

    const reader = proc.stdout.getReader()
    const decoder = new TextDecoder()
    let buffer = ''

    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      const lines = buffer.split('\n')
      buffer = lines.pop() || ''

      for (const line of lines) {
        const clean = line.trim()
        if (!clean) continue

        // Detect track header: e.g. "Artist — Title"
        if (clean.includes(' — ') && !clean.startsWith('Album:')) {
          const parts = clean.split(' — ')
          artist = parts[0]?.replace(/^\[\d+\/\d+\]\s*/, '').trim() || artist
          title = parts[1]?.trim() || title
          onProgress?.(`Ripping ${artist} — ${title}...`)
        } else if (clean.startsWith('Album:')) {
          onProgress?.(`Ripping ${clean}...`)
        }
      }
    }

    const exitCode = await proc.exited
    if (exitCode !== 0) {
      const stderr = await new Response(proc.stderr).text()
      throw new Error(
        `Ripper failed with exit code ${exitCode}: ${stderr.trim() || 'Unknown error'}`,
      )
    }

    // Locate the downloaded file in output directory
    const audioFile = findLatestM4aFile(this.outputDir)
    if (!audioFile) {
      throw new Error('Rip completed but output audio file was not found')
    }

    return {
      filePath: audioFile,
      title,
      artist,
    }
  }
}

export const defaultRipper = new AppleBruhRipper()
