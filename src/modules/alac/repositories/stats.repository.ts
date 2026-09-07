import { and, avg, count, desc, eq } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { requests, tracks } from '@/db/schema.ts'
import { debugSpan } from '@/utils/logger.ts'

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

export interface IStatsRepository {
  getStats(): Promise<AlacStats>
}

export class StatsRepository implements IStatsRepository {
  private readonly _db?: AppDatabase

  constructor(db?: AppDatabase) {
    this._db = db
  }

  private get db(): AppDatabase {
    return this._db ?? defaultDb
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

export const statsRepository = new StatsRepository()
