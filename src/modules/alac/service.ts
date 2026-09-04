import { eq } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { type NewRequest, requests, type Track, tracks } from '@/db/schema.ts'

export interface SaveTrackInput {
  appleTrackId: string
  messageId: number
  fileId: string
  fileUniqueId?: string
}

export interface IAlacService {
  findCachedTrack(appleTrackId: string): Promise<Track | null>
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
    const [track] = await this.db
      .select()
      .from(tracks)
      .where(eq(tracks.appleTrackId, appleTrackId))
      .limit(1)

    return track ?? null
  }

  async saveTrack(input: SaveTrackInput): Promise<Track> {
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
    const deleted = await this.db
      .delete(tracks)
      .where(eq(tracks.appleTrackId, appleTrackId))
      .returning()

    return deleted.length > 0
  }

  async logRequest(data: NewRequest): Promise<void> {
    await this.db.insert(requests).values(data)
  }
}

export const alacService = new AlacService()
