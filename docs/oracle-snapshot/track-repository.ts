import { eq, inArray, notInArray, sql } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { type Track, tracks } from '@/db/schema.ts'
import { debug, debugSpan } from '@/utils/logger.ts'

export interface SaveTrackInput {
  appleTrackId: string
  messageId: number
  fileId: string
  fileUniqueId: string
  title: string
  artist: string
  album: string
  duration: number
  bitDepth: number
  sampleRate: number
  genre: string
  releaseDate: string
  trackNumber: number
  trackCount: number
}

export interface ITrackRepository {
  findCachedTrack(appleTrackId: string): Promise<Track | null>
  findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>>
  findTrackByFileUniqueId(fileUniqueId: string): Promise<Track | null>
  searchCachedTracks(query: string, limit?: number): Promise<Track[]>
  saveTrack(input: SaveTrackInput): Promise<Track>
  deleteTrack(appleTrackId: string): Promise<boolean>
  getAllTrackIds(): Promise<string[]>
  deleteTracksNotIn(validTrackIds: string[]): Promise<number>
}

export class TrackRepository implements ITrackRepository {
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

  async findTrackByFileUniqueId(fileUniqueId: string): Promise<Track | null> {
    using _ = debugSpan('db_find_track_by_file_unique_id', {
      fileUniqueId,
    }).enter()

    const [track] = await this.db
      .select()
      .from(tracks)
      .where(eq(tracks.fileUniqueId, fileUniqueId))
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

  async searchCachedTracks(query: string, limit = 10): Promise<Track[]> {
    using _ = debugSpan('db_search_cached_tracks', { query, limit }).enter()

    const trimmed = query.trim()
    if (!trimmed) {
      return []
    }

    const pattern = `%${trimmed}%`
    const threshold = 0.35
    const rows = await this.db
      .select()
      .from(tracks)
      .where(
        sql`(${tracks.appleTrackId} = ${trimmed}
          OR ${tracks.title} ILIKE ${pattern}
          OR ${tracks.artist} ILIKE ${pattern}
          OR ${tracks.album} ILIKE ${pattern}
          OR word_similarity(${trimmed}, ${tracks.title} || ' ' || ${tracks.artist} || ' ' || ${tracks.album}) >= ${threshold})`,
      )
      .orderBy(
        sql`CASE
          WHEN ${tracks.appleTrackId} = ${trimmed} THEN 3
          WHEN (${tracks.title} ILIKE ${pattern} OR ${tracks.artist} ILIKE ${pattern} OR ${tracks.album} ILIKE ${pattern}) THEN 2
          ELSE 1
        END DESC`,
        sql`word_similarity(${trimmed}, ${tracks.title} || ' ' || ${tracks.artist} || ' ' || ${tracks.album}) DESC`,
      )
      .limit(limit)

    debug('Track search query completed', {
      query: trimmed,
      matches: rows.length,
    })

    return rows
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
        title: input.title,
        artist: input.artist,
        album: input.album,
        duration: input.duration,
        bitDepth: input.bitDepth,
        sampleRate: input.sampleRate,
        genre: input.genre,
        releaseDate: input.releaseDate,
        trackNumber: input.trackNumber,
        trackCount: input.trackCount,
        updatedAt: new Date(),
      })
      .onConflictDoUpdate({
        target: tracks.appleTrackId,
        set: {
          messageId: input.messageId,
          fileId: input.fileId,
          fileUniqueId: input.fileUniqueId,
          title: input.title,
          artist: input.artist,
          album: input.album,
          duration: input.duration,
          bitDepth: input.bitDepth,
          sampleRate: input.sampleRate,
          genre: input.genre,
          releaseDate: input.releaseDate,
          trackNumber: input.trackNumber,
          trackCount: input.trackCount,
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

  async getAllTrackIds(): Promise<string[]> {
    using _ = debugSpan('db_get_all_track_ids').enter()

    const rows = await this.db
      .select({ appleTrackId: tracks.appleTrackId })
      .from(tracks)

    return rows.map((r) => r.appleTrackId)
  }

  async deleteTracksNotIn(validTrackIds: string[]): Promise<number> {
    using _ = debugSpan('db_delete_tracks_not_in', {
      validCount: validTrackIds.length,
    }).enter()

    if (validTrackIds.length === 0) {
      const deleted = await this.db.delete(tracks).returning()
      return deleted.length
    }

    const deleted = await this.db
      .delete(tracks)
      .where(notInArray(tracks.appleTrackId, validTrackIds))
      .returning()

    return deleted.length
  }
}

export const trackRepository = new TrackRepository()
