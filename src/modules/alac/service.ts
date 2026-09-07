import type { AppDatabase } from '@/db/index.ts'
import type { NewRequest, Track } from '@/db/schema.ts'

import {
  type AlacStats,
  type IRequestLogRepository,
  type IStatsRepository,
  type ITrackRepository,
  RequestLogRepository,
  type SaveTrackInput,
  StatsRepository,
  type TopTrackStat,
  TrackRepository,
} from './repositories/index.ts'

export type { AlacStats, SaveTrackInput, TopTrackStat }

export interface IAlacService {
  findCachedTrack(appleTrackId: string): Promise<Track | null>
  findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>>
  findTrackByFileUniqueId(fileUniqueId: string): Promise<Track | null>
  searchCachedTracks(query: string, limit?: number): Promise<Track[]>
  saveTrack(input: SaveTrackInput): Promise<Track>
  deleteTrack(appleTrackId: string): Promise<boolean>
  getAllTrackIds(): Promise<string[]>
  deleteTracksNotIn(validTrackIds: string[]): Promise<number>
  logRequest(data: NewRequest): Promise<void>
  getStats(): Promise<AlacStats>
}

export class AlacService implements IAlacService {
  readonly tracks: ITrackRepository
  readonly requestLogs: IRequestLogRepository
  readonly stats: IStatsRepository

  constructor(
    db?: AppDatabase,
    tracks?: ITrackRepository,
    requestLogs?: IRequestLogRepository,
    stats?: IStatsRepository,
  ) {
    this.tracks = tracks ?? new TrackRepository(db)
    this.requestLogs = requestLogs ?? new RequestLogRepository(db)
    this.stats = stats ?? new StatsRepository(db)
  }

  findCachedTrack(appleTrackId: string): Promise<Track | null> {
    return this.tracks.findCachedTrack(appleTrackId)
  }

  findTrackByFileUniqueId(fileUniqueId: string): Promise<Track | null> {
    return this.tracks.findTrackByFileUniqueId(fileUniqueId)
  }

  findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>> {
    return this.tracks.findCachedTracks(appleTrackIds)
  }

  searchCachedTracks(query: string, limit = 10): Promise<Track[]> {
    return this.tracks.searchCachedTracks(query, limit)
  }

  saveTrack(input: SaveTrackInput): Promise<Track> {
    return this.tracks.saveTrack(input)
  }

  deleteTrack(appleTrackId: string): Promise<boolean> {
    return this.tracks.deleteTrack(appleTrackId)
  }

  getAllTrackIds(): Promise<string[]> {
    return this.tracks.getAllTrackIds()
  }

  deleteTracksNotIn(validTrackIds: string[]): Promise<number> {
    return this.tracks.deleteTracksNotIn(validTrackIds)
  }

  logRequest(data: NewRequest): Promise<void> {
    return this.requestLogs.logRequest(data)
  }

  getStats(): Promise<AlacStats> {
    return this.stats.getStats()
  }
}

export const alacService = new AlacService()
