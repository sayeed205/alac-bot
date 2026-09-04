import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'bun:test'

import { eq } from 'drizzle-orm'

import { DbDumpService } from '@/db/dump.ts'
import type { AppDatabase } from '@/db/index.ts'
import * as schema from '@/db/schema.ts'

import { setupTestDb } from '../test-db.ts'

describe('DbDumpService (PostgreSQL Integration)', () => {
  let db: AppDatabase
  let cleanDb: () => Promise<void>
  let close: () => Promise<void>
  let dumpService: DbDumpService

  beforeAll(async () => {
    const testEnv = await setupTestDb()
    db = testEnv.db
    cleanDb = testEnv.cleanDb
    close = testEnv.close
    dumpService = new DbDumpService(db)
  })

  beforeEach(async () => {
    await cleanDb()
  })

  afterAll(async () => {
    await close()
  })

  it('exports an empty database dump successfully', async () => {
    const dump = await dumpService.exportDump()
    expect(dump.filename).toContain('alac_dump_')
    expect(dump.filename).toContain('.sql.gz')
    expect(dump.buffer.length).toBeGreaterThan(0)
    expect(dump.stats.usersCount).toBe(0)
    expect(dump.stats.tracksCount).toBe(0)
    expect(dump.stats.requestsCount).toBe(0)
  })

  it('exports database with users, tracks, and requests with correct SQL escaping', async () => {
    await db.insert(schema.users).values({
      telegramId: 12345,
      name: "O'Connor Admin",
    })

    await db.insert(schema.tracks).values({
      appleTrackId: 'apple_1',
      messageId: 50,
      fileId: 'file_abc',
      fileUniqueId: 'uniq_abc',
      title: "Don't Stop Believin'",
      artist: 'Journey',
      album: 'Escape',
      duration: 250,
      bitDepth: 24,
      sampleRate: 48000,
      genre: 'Classic Rock',
      releaseDate: '1981-07-31',
      trackNumber: 1,
      trackCount: 10,
    })

    await db.insert(schema.requests).values({
      telegramId: 12345,
      chatId: -1001234,
      appleTrackId: 'apple_1',
      isCacheHit: true,
      durationMs: 42,
      status: 'completed',
      errorReason: null,
    })

    const dump = await dumpService.exportDump()
    expect(dump.stats.usersCount).toBe(1)
    expect(dump.stats.tracksCount).toBe(1)
    expect(dump.stats.requestsCount).toBe(1)

    const decompressed = Bun.gunzipSync(Buffer.from(dump.buffer))
    const sqlText = new TextDecoder().decode(decompressed)

    expect(sqlText).toContain("O''Connor Admin")
    expect(sqlText).toContain("Don''t Stop Believin''")
    expect(sqlText).toContain('ON CONFLICT (apple_track_id) DO UPDATE')
  })

  it('imports and restores data into a clean database', async () => {
    await db.insert(schema.users).values({
      telegramId: 99999,
      name: 'Dump User',
    })

    await db.insert(schema.tracks).values({
      appleTrackId: 'apple_dump_track',
      messageId: 100,
      fileId: 'file_dump',
      fileUniqueId: 'uniq_dump',
      title: 'Restored Song',
      artist: 'Restored Artist',
      album: 'Restored Album',
      duration: 180,
      bitDepth: 16,
      sampleRate: 44100,
      genre: 'Pop',
      releaseDate: '2020-01-01',
      trackNumber: 1,
      trackCount: 1,
    })

    const dump = await dumpService.exportDump()

    await cleanDb()
    const usersBefore = await db.select().from(schema.users)
    expect(usersBefore.length).toBe(0)

    const restoreStats = await dumpService.importDump(dump.buffer)
    expect(restoreStats.usersMerged).toBe(1)
    expect(restoreStats.tracksMerged).toBe(1)

    const usersAfter = await db.select().from(schema.users)
    expect(usersAfter.length).toBe(1)
    expect(usersAfter[0]?.name).toBe('Dump User')

    const tracksAfter = await db.select().from(schema.tracks)
    expect(tracksAfter.length).toBe(1)
    expect(tracksAfter[0]?.appleTrackId).toBe('apple_dump_track')
    expect(tracksAfter[0]?.title).toBe('Restored Song')
  })

  it('upserts and merges existing records on conflict', async () => {
    await db.insert(schema.tracks).values({
      appleTrackId: 'track_conflict',
      messageId: 10,
      fileId: 'old_file',
      fileUniqueId: 'uniq_c',
      title: 'Old Title',
      artist: 'Artist',
      album: 'Album',
      duration: 200,
      bitDepth: 16,
      sampleRate: 44100,
      genre: 'Rock',
      releaseDate: '2020',
      trackNumber: 1,
      trackCount: 1,
    })

    const dump = await dumpService.exportDump()

    await db
      .update(schema.tracks)
      .set({ title: 'Mutated Title', messageId: 99 })
      .where(eq(schema.tracks.appleTrackId, 'track_conflict'))

    await dumpService.importDump(dump.buffer)

    const restored = await db
      .select()
      .from(schema.tracks)
      .where(eq(schema.tracks.appleTrackId, 'track_conflict'))

    expect(restored.length).toBe(1)
    expect(restored[0]?.messageId).toBe(10)
    expect(restored[0]?.title).toBe('Old Title')
  })

  it('rolls back completely when import fails due to corrupted or invalid SQL', async () => {
    const invalidSql =
      'INSERT INTO users (telegram_id, name) VALUES (1, "Valid");\nMALFORMED SQL STATEMENT SYNTAX ERROR;'
    const compressed = Bun.gzipSync(Buffer.from(invalidSql))

    let threw = false
    try {
      await dumpService.importDump(compressed)
    } catch {
      threw = true
    }

    expect(threw).toBe(true)

    const users = await db.select().from(schema.users)
    expect(users.length).toBe(0)
  })
})
