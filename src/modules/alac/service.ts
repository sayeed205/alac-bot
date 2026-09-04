import { and, avg, count, desc, eq, inArray } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { type NewRequest, requests, type Track, tracks } from '@/db/schema.ts'
import { debug, debugSpan } from '@/utils/logger.ts'

export interface SaveTrackInput {
  appleTrackId: string
  messageId: number
  fileId: string
  fileUniqueId?: string
}

export interface TopTrackStat {
  appleTrackId: string
  requestCount: number
}

export interface AlacStats {
  totalCachedTracks: number
  totalRequests: number
  cacheHits: number
  cacheMisses: number
  cacheHitRatio: number
  avgRipDurationMs: number
  avgCacheDurationMs: number
  totalFailedRequests: number
  topTracks: TopTrackStat[]
}

export interface IAlacService {
  findCachedTrack(appleTrackId: string): Promise<Track | null>
  findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>>
  saveTrack(input: SaveTrackInput): Promise<Track>
  deleteTrack(appleTrackId: string): Promise<boolean>
  logRequest(data: NewRequest): Promise<void>
  getStats(): Promise<AlacStats>
}

export class AlacService implements IAlacService {
  private readonly _db?: AppDatabase

  constructor(db?: AppDatabase) {
    this._db = db
  }

  private get db(): AppDatabase {
    return this._db ?? defaultDb
  }

  async findCachedTrack(appleTrackId: string): Promise<Track | null> {
    using _ = debugSpan('db_find_cached_track', { appleTrackId }).enter()

    const [track] = await this.db
      .select()
      .from(tracks)
      .where(eq(tracks.appleTrackId, appleTrackId))
      .limit(1)

    return track ?? null
  }

  async findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>> {
    using _ = debugSpan('db_find_cached_tracks', {
      count: appleTrackIds.length,
    }).enter()

    const result = new Map<string, Track>()
    const uniqueIds = Array.from(new Set(appleTrackIds.filter(Boolean)))
    if (uniqueIds.length === 0) {
      return result
    }

    const rows = await this.db
      .select()
      .from(tracks)
      .where(inArray(tracks.appleTrackId, uniqueIds))

    for (const row of rows) {
      result.set(row.appleTrackId, row)
    }

    debug('Batch cache lookup completed', {
      queried: uniqueIds.length,
      found: result.size,
    })

    return result
  }

  async saveTrack(input: SaveTrackInput): Promise<Track> {
    using _ = debugSpan('db_save_track', {
      appleTrackId: input.appleTrackId,
      messageId: input.messageId,
    }).enter()

    const [saved] = await this.db
      .insert(tracks)
      .values({
        appleTrackId: input.appleTrackId,
        messageId: input.messageId,
        fileId: input.fileId,
        fileUniqueId: input.fileUniqueId,
        updatedAt: new Date(),
      })
      .onConflictDoUpdate({
        target: tracks.appleTrackId,
        set: {
          messageId: input.messageId,
          fileId: input.fileId,
          fileUniqueId: input.fileUniqueId,
          updatedAt: new Date(),
        },
      })
      .returning()

    if (!saved) {
      throw new Error('Failed to save or update track in database')
    }

    return saved
  }

  async deleteTrack(appleTrackId: string): Promise<boolean> {
    using _ = debugSpan('db_delete_track', { appleTrackId }).enter()

    const deleted = await this.db
      .delete(tracks)
      .where(eq(tracks.appleTrackId, appleTrackId))
      .returning()

    return deleted.length > 0
  }

  async logRequest(data: NewRequest): Promise<void> {
    using _ = debugSpan('db_log_request', {
      appleTrackId: data.appleTrackId,
      status: data.status,
    }).enter()

    await this.db.insert(requests).values(data)
  }

  async getStats(): Promise<AlacStats> {
    using _ = debugSpan('db_get_stats').enter()

    const [tracksCount] = await this.db.select({ value: count() }).from(tracks)

    const [totalReqs] = await this.db.select({ value: count() }).from(requests)

    const [cacheHits] = await this.db
      .select({ value: count() })
      .from(requests)
      .where(eq(requests.isCacheHit, true))

    const [failedReqs] = await this.db
      .select({ value: count() })
      .from(requests)
      .where(eq(requests.status, 'failed'))

    const [avgRip] = await this.db
      .select({ value: avg(requests.durationMs) })
      .from(requests)
      .where(
        and(eq(requests.isCacheHit, false), eq(requests.status, 'completed')),
      )

    const [avgCache] = await this.db
      .select({ value: avg(requests.durationMs) })
      .from(requests)
      .where(
        and(eq(requests.isCacheHit, true), eq(requests.status, 'completed')),
      )

    const topTracksRows = await this.db
      .select({
        appleTrackId: requests.appleTrackId,
        requestCount: count(),
      })
      .from(requests)
      .where(eq(requests.status, 'completed'))
      .groupBy(requests.appleTrackId)
      .orderBy(desc(count()))
      .limit(5)

    const totalRequests = Number(totalReqs?.value) || 0
    const hits = Number(cacheHits?.value) || 0
    const misses = totalRequests - hits
    const hitRatio = totalRequests > 0 ? (hits / totalRequests) * 100 : 0

    return {
      totalCachedTracks: Number(tracksCount?.value) || 0,
      totalRequests,
      cacheHits: hits,
      cacheMisses: misses,
      cacheHitRatio: Math.round(hitRatio * 10) / 10,
      avgRipDurationMs: Math.round(Number(avgRip?.value) || 0),
      avgCacheDurationMs: Math.round(Number(avgCache?.value) || 0),
      totalFailedRequests: Number(failedReqs?.value) || 0,
      topTracks: topTracksRows.map((r) => ({
        appleTrackId: r.appleTrackId,
        requestCount: Number(r.requestCount) || 0,
      })),
    }
  }
}

export const alacService = new AlacService()
