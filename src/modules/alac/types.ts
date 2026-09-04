export interface AppleTrackMetadata {
  id: string
  title: string
  artist: string
  album: string
  albumArtist: string
  genre?: string
  releaseDate?: string
  composer?: string
  trackNumber?: number
  trackCount?: number
  discNumber?: number
  discCount?: number
  duration: number // in seconds
  explicit: boolean
  artworkUrl?: string
}

export interface TrackRipResult {
  filePath: string
  title: string
  artist: string
  album?: string
  duration?: number
  bitDepth?: string
  sampleRate?: string
  codec?: string
}
