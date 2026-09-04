import { eq, inArray } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { type NewRequest, requests, type Track, tracks } from '@/db/schema.ts'
import { debug, debugSpan } from '@/utils/logger.ts'

export interface SaveTrackInput {
  appleTrackId: string
  messageId: number
  fileId: string
  fileUniqueId?: string
}

export interface IAlacService {
  findCachedTrack(appleTrackId: string): Promise<Track | null>
  findCachedTracks(appleTrackIds: string[]): Promise<Map<string, Track>>
  saveTrack(input: SaveTrackInput): Promise<Track>
  deleteTrack(appleTrackId: string): Promise<boolean>
  logRequest(data: NewRequest): Promise<void>
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
}

export const alacService = new AlacService()
