import { afterAll, beforeAll, describe, expect, it } from 'bun:test'

import { PGlite } from '@electric-sql/pglite'
import { eq } from 'drizzle-orm'
import { drizzle, type PgliteDatabase } from 'drizzle-orm/pglite'
import { migrate } from 'drizzle-orm/pglite/migrator'

import * as schema from '@/db/schema.ts'

describe('Drizzle Schema & Database Constraints (PGlite Integration)', () => {
  let client: PGlite
  let db: PgliteDatabase<typeof schema>

  beforeAll(async () => {
    client = new PGlite()
    await client.waitReady
    db = drizzle(client, { schema })
    await migrate(db, { migrationsFolder: './drizzle' })
  })

  afterAll(async () => {
    if (client && !client.closed) {
      await client.close()
    }
  })

  describe('Users Table', () => {
    it('inserts and queries a user with default timestamp', async () => {
      const inserted = await db
        .insert(schema.users)
        .values({
          telegramId: 12345678,
          name: 'Alice Developer',
        })
        .returning()

      expect(inserted.length).toBe(1)
      const user = inserted[0]
      expect(user?.telegramId).toBe(12345678)
      expect(user?.name).toBe('Alice Developer')
      expect(user?.createdAt).toBeInstanceOf(Date)

      const found = await db
        .select()
        .from(schema.users)
        .where(eq(schema.users.telegramId, 12345678))

      expect(found.length).toBe(1)
      expect(found[0]?.name).toBe('Alice Developer')
    })

    it('enforces primary key constraint on duplicate telegramId', async () => {
      let threw = false
      try {
        await db.insert(schema.users).values({
          telegramId: 12345678,
          name: 'Duplicate Alice',
        })
      } catch {
        threw = true
      }
      expect(threw).toBe(true)
    })
  })

  describe('Tracks Table', () => {
    it('inserts a complete track record with all required non-null fields', async () => {
      const inserted = await db
        .insert(schema.tracks)
        .values({
          appleTrackId: 'apple_101',
          messageId: 42,
          fileId: 'file_id_101',
          fileUniqueId: 'uniq_101',
          title: 'Masterpiece',
          artist: 'Great Artist',
          album: 'Debut Album',
          duration: 210,
          bitDepth: 24,
          sampleRate: 96000,
          genre: 'Rock',
          releaseDate: '2023-05-12',
          trackNumber: 1,
          trackCount: 10,
        })
        .returning()

      expect(inserted.length).toBe(1)
      const track = inserted[0]
      expect(track?.id).toBeDefined()
      expect(track?.appleTrackId).toBe('apple_101')
      expect(track?.bitDepth).toBe(24)
      expect(track?.sampleRate).toBe(96000)
      expect(track?.createdAt).toBeInstanceOf(Date)
      expect(track?.updatedAt).toBeInstanceOf(Date)
    })

    it('enforces unique constraint on appleTrackId', async () => {
      let threw = false
      try {
        await db.insert(schema.tracks).values({
          appleTrackId: 'apple_101',
          messageId: 99,
          fileId: 'other_file',
          fileUniqueId: 'other_uniq',
          title: 'Duplicate Track',
          artist: 'Artist',
          album: 'Album',
          duration: 180,
          bitDepth: 16,
          sampleRate: 44100,
          genre: 'Pop',
          releaseDate: '2023',
          trackNumber: 1,
          trackCount: 1,
        })
      } catch {
        threw = true
      }
      expect(threw).toBe(true)
    })

    it('enforces notNull constraints when required fields are missing', async () => {
      let threw = false
      try {
        await db.insert(schema.tracks).values({
          appleTrackId: 'apple_missing_fields',
          messageId: 50,
          fileId: 'file_id',
          fileUniqueId: 'uniq_id',
          duration: 100,
        } as never)
      } catch {
        threw = true
      }
      expect(threw).toBe(true)
    })

    it('supports onConflictDoUpdate upsert pattern recommended by Drizzle', async () => {
      await db
        .insert(schema.tracks)
        .values({
          appleTrackId: 'apple_101',
          messageId: 500,
          fileId: 'updated_file_id',
          fileUniqueId: 'uniq_101',
          title: 'Masterpiece (Remastered)',
          artist: 'Great Artist',
          album: 'Debut Album',
          duration: 215,
          bitDepth: 24,
          sampleRate: 96000,
          genre: 'Rock',
          releaseDate: '2023-05-12',
          trackNumber: 1,
          trackCount: 10,
        })
        .onConflictDoUpdate({
          target: schema.tracks.appleTrackId,
          set: {
            messageId: 500,
            fileId: 'updated_file_id',
            title: 'Masterpiece (Remastered)',
            duration: 215,
          },
        })

      const updated = await db
        .select()
        .from(schema.tracks)
        .where(eq(schema.tracks.appleTrackId, 'apple_101'))

      expect(updated.length).toBe(1)
      expect(updated[0]?.messageId).toBe(500)
      expect(updated[0]?.fileId).toBe('updated_file_id')
      expect(updated[0]?.title).toBe('Masterpiece (Remastered)')
    })
  })

  describe('Requests Analytics Table', () => {
    it('records analytics requests with optional and required fields', async () => {
      const inserted = await db
        .insert(schema.requests)
        .values({
          telegramId: 12345678,
          chatId: -100123456789,
          appleTrackId: 'apple_101',
          isCacheHit: true,
          durationMs: 45,
          status: 'completed',
          errorReason: null,
        })
        .returning()

      expect(inserted.length).toBe(1)
      expect(inserted[0]?.durationMs).toBe(45)
      expect(inserted[0]?.errorReason).toBeNull()
      expect(inserted[0]?.createdAt).toBeInstanceOf(Date)
    })

    it('records failed requests with error reason', async () => {
      const inserted = await db
        .insert(schema.requests)
        .values({
          telegramId: 12345678,
          chatId: -100123456789,
          appleTrackId: 'apple_999',
          isCacheHit: false,
          durationMs: 12000,
          status: 'failed',
          errorReason: 'Decryption key unavailable',
        })
        .returning()

      expect(inserted[0]?.status).toBe('failed')
      expect(inserted[0]?.errorReason).toBe('Decryption key unavailable')
    })
  })

  describe('Transaction & Rollback Testing', () => {
    it('rolls back database modifications when transaction throws', async () => {
      const initialCount = await db.select().from(schema.users)

      try {
        await db.transaction(async (tx) => {
          await tx.insert(schema.users).values({
            telegramId: 99999999,
            name: 'Temporary User',
          })
          throw new Error('Forced rollback error')
        })
      } catch {}

      const afterCount = await db.select().from(schema.users)
      expect(afterCount.length).toBe(initialCount.length)

      const rolledBackUser = await db
        .select()
        .from(schema.users)
        .where(eq(schema.users.telegramId, 99999999))

      expect(rolledBackUser.length).toBe(0)
    })
  })
})
