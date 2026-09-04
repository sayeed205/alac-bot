import { afterAll, beforeAll, describe, expect, it } from 'bun:test'

import { PGlite } from '@electric-sql/pglite'
import { eq } from 'drizzle-orm'
import { drizzle, type PgliteDatabase } from 'drizzle-orm/pglite'
import { migrate } from 'drizzle-orm/pglite/migrator'

import { DbDumpService } from '@/db/dump.ts'
import type { AppDatabase } from '@/db/index.ts'
import * as schema from '@/db/schema.ts'

describe('DbDumpService (PGlite Integration)', () => {
  let client: PGlite
  let db: PgliteDatabase<typeof schema>
  let dumpService: DbDumpService

  beforeAll(async () => {
    client = new PGlite()
    await client.waitReady
    db = drizzle(client, { schema })
    await migrate(db, { migrationsFolder: './drizzle' })
    dumpService = new DbDumpService(db as unknown as AppDatabase)
  })

  afterAll(async () => {
    if (client && !client.closed) {
      await client.close()
    }
  })

  it('exports an empty database dump successfully', async () => {
    const { buffer, filename, stats } = await dumpService.exportDump()

    expect(filename).toMatch(
      /^alac_dump_\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}\.sql\.gz$/,
    )
    expect(stats.usersCount).toBe(0)
    expect(stats.tracksCount).toBe(0)
    expect(stats.requestsCount).toBe(0)
    expect(stats.bytes).toBeGreaterThan(0)
    expect(buffer.length).toBe(stats.bytes)

    const decompressed = Bun.gunzipSync(Buffer.from(buffer))
    const sqlText = new TextDecoder().decode(decompressed)
    expect(sqlText).toContain('-- ALAC Telegram Bot Database Dump')
    expect(sqlText).toContain(
      "SELECT setval(pg_get_serial_sequence('tracks', 'id')",
    )
  })

  it('exports database with users, tracks, and requests with correct SQL escaping', async () => {
    await db.insert(schema.users).values({
      telegramId: 111222,
      name: "O'Connor Admin",
      createdAt: new Date('2025-01-01T12:00:00Z'),
    })

    await db.insert(schema.tracks).values({
      appleTrackId: 'track_123',
      messageId: 50,
      fileId: 'tg_file_abc',
      fileUniqueId: 'uniq_abc',
      title: "Rockin' All Over",
      artist: 'Status Quo',
      album: 'Pictures',
      duration: 180,
      bitDepth: 24,
      sampleRate: 48000,
      genre: 'Rock',
      releaseDate: '1977-11-01',
      trackNumber: 2,
      trackCount: 10,
    })

    await db.insert(schema.requests).values({
      telegramId: 111222,
      chatId: -100987654321,
      appleTrackId: 'track_123',
      isCacheHit: false,
      durationMs: 450,
      status: 'completed',
      errorReason: null,
    })

    const { buffer, stats } = await dumpService.exportDump()
    expect(stats.usersCount).toBe(1)
    expect(stats.tracksCount).toBe(1)
    expect(stats.requestsCount).toBe(1)

    const decompressed = Bun.gunzipSync(Buffer.from(buffer))
    const sqlText = new TextDecoder().decode(decompressed)

    expect(sqlText).toContain("'O''Connor Admin'")
    expect(sqlText).toContain("'Rockin'' All Over'")
    expect(sqlText).toContain('ON CONFLICT (telegram_id) DO UPDATE')
    expect(sqlText).toContain('ON CONFLICT (apple_track_id) DO UPDATE')
  })

  it('imports and restores data into a clean database', async () => {
    const { buffer } = await dumpService.exportDump()

    const freshClient = new PGlite()
    await freshClient.waitReady
    const freshDb = drizzle(freshClient, { schema })
    await migrate(freshDb, { migrationsFolder: './drizzle' })
    const freshDumpService = new DbDumpService(
      freshDb as unknown as AppDatabase,
    )

    const restoreStats = await freshDumpService.importDump(buffer)
    expect(restoreStats.usersMerged).toBe(1)
    expect(restoreStats.tracksMerged).toBe(1)
    expect(restoreStats.requestsMerged).toBe(1)
    expect(restoreStats.durationMs).toBeGreaterThanOrEqual(0)

    const users = await freshDb.select().from(schema.users)
    expect(users.length).toBe(1)
    expect(users[0]?.telegramId).toBe(111222)
    expect(users[0]?.name).toBe("O'Connor Admin")

    const tracks = await freshDb.select().from(schema.tracks)
    expect(tracks.length).toBe(1)
    expect(tracks[0]?.appleTrackId).toBe('track_123')
    expect(tracks[0]?.title).toBe("Rockin' All Over")

    const requests = await freshDb.select().from(schema.requests)
    expect(requests.length).toBe(1)
    expect(requests[0]?.appleTrackId).toBe('track_123')

    const newTrack = await freshDb
      .insert(schema.tracks)
      .values({
        appleTrackId: 'new_seq_track',
        messageId: 51,
        fileId: 'fid_2',
        fileUniqueId: 'uniq_2',
        title: 'Next Track',
        artist: 'Artist',
        album: 'Album',
        duration: 100,
        bitDepth: 16,
        sampleRate: 44100,
        genre: 'Pop',
        releaseDate: '2020-01-01',
        trackNumber: 1,
        trackCount: 1,
      })
      .returning()

    expect(newTrack.length).toBe(1)
    expect(newTrack[0]?.id).toBeGreaterThan(tracks[0]?.id ?? 0)

    await freshClient.close()
  })

  it('upserts and merges existing records on conflict', async () => {
    const freshClient = new PGlite()
    await freshClient.waitReady
    const freshDb = drizzle(freshClient, { schema })
    await migrate(freshDb, { migrationsFolder: './drizzle' })
    const freshDumpService = new DbDumpService(
      freshDb as unknown as AppDatabase,
    )

    await freshDb.insert(schema.users).values({
      telegramId: 111222,
      name: 'Old Name',
      createdAt: new Date('2024-01-01T00:00:00Z'),
    })

    await freshDb.insert(schema.tracks).values({
      appleTrackId: 'track_123',
      messageId: 10,
      fileId: 'old_fid',
      fileUniqueId: 'old_uniq',
      title: 'Old Title',
      artist: 'Old Artist',
      album: 'Old Album',
      duration: 100,
      bitDepth: 16,
      sampleRate: 44100,
      genre: 'Old Genre',
      releaseDate: '2000-01-01',
      trackNumber: 1,
      trackCount: 1,
    })

    const { buffer } = await dumpService.exportDump()
    await freshDumpService.importDump(buffer)

    const updatedUser = await freshDb
      .select()
      .from(schema.users)
      .where(eq(schema.users.telegramId, 111222))
    expect(updatedUser[0]?.name).toBe("O'Connor Admin")

    const updatedTrack = await freshDb
      .select()
      .from(schema.tracks)
      .where(eq(schema.tracks.appleTrackId, 'track_123'))
    expect(updatedTrack[0]?.title).toBe("Rockin' All Over")
    expect(updatedTrack[0]?.fileId).toBe('tg_file_abc')

    await freshClient.close()
  })

  it('rolls back completely when import fails due to corrupted or invalid SQL', async () => {
    const invalidSql =
      'INSERT INTO users (telegram_id) VALUES (999999);\nINVALID SQL SYNTAX HERE;\n'
    const corruptedBuffer = Bun.gzipSync(Buffer.from(invalidSql))

    let threw = false
    try {
      await dumpService.importDump(corruptedBuffer)
    } catch {
      threw = true
    }
    expect(threw).toBe(true)

    const users = await db
      .select()
      .from(schema.users)
      .where(eq(schema.users.telegramId, 999999))
    expect(users.length).toBe(0)
  })
})
