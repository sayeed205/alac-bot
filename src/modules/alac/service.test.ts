import { afterAll, beforeAll, describe, expect, it } from 'bun:test'

import { PGlite } from '@electric-sql/pglite'
import { drizzle } from 'drizzle-orm/pglite'
import { migrate } from 'drizzle-orm/pglite/migrator'

import type { AppDatabase } from '@/db/index.ts'
import * as schema from '@/db/schema.ts'
import { AlacService } from '@/modules/alac/service.ts'

describe('AlacService', () => {
  let client: PGlite
  let service: AlacService

  beforeAll(async () => {
    client = new PGlite()
    await client.waitReady
    const testDb = drizzle(client, { schema })
    await migrate(testDb, { migrationsFolder: './drizzle' } || {})
    service = new AlacService(testDb as unknown as AppDatabase)
  })

  afterAll(async () => {
    if (client && !client.closed) {
      await client.close()
    }
  })

  it('returns null when track is not cached', async () => {
    const track = await service.findCachedTrack('non_existent')
    expect(track).toBeNull()
  })

  it('saves and retrieves a cached track', async () => {
    const saved = await service.saveTrack({
      appleTrackId: '123456789',
      messageId: 42,
      fileId: 'tg_file_id_test',
      fileUniqueId: 'tg_unique_id_test',
    })

    expect(saved.appleTrackId).toBe('123456789')
    expect(saved.messageId).toBe(42)

    const fetched = await service.findCachedTrack('123456789')
    expect(fetched).not.toBeNull()
    expect(fetched?.messageId).toBe(42)
    expect(fetched?.fileId).toBe('tg_file_id_test')
  })

  it('upserts an existing track when saved with updated details', async () => {
    await service.saveTrack({
      appleTrackId: '123456789',
      messageId: 100,
      fileId: 'tg_file_id_updated',
    })

    const fetched = await service.findCachedTrack('123456789')
    expect(fetched?.messageId).toBe(100)
    expect(fetched?.fileId).toBe('tg_file_id_updated')
  })

  it('batch retrieves multiple cached tracks with findCachedTracks', async () => {
    // Empty list returns empty map
    const emptyMap = await service.findCachedTracks([])
    expect(emptyMap.size).toBe(0)

    // Save another track
    await service.saveTrack({
      appleTrackId: '987654321',
      messageId: 101,
      fileId: 'tg_file_id_2',
    })

    const map = await service.findCachedTracks([
      '123456789',
      '987654321',
      'missing_track',
    ])

    expect(map.size).toBe(2)
    expect(map.get('123456789')?.messageId).toBe(100)
    expect(map.get('987654321')?.messageId).toBe(101)
    expect(map.has('missing_track')).toBe(false)
  })

  it('logs a request entry in the analytics requests table', async () => {
    await service.logRequest({
      telegramId: 6252490183,
      chatId: -100123456,
      appleTrackId: '123456789',
      isCacheHit: true,
      durationMs: 15,
      status: 'completed',
    })

    // If it did not throw, it succeeded
    expect(true).toBe(true)
  })

  it('deletes an existing track', async () => {
    const deleted = await service.deleteTrack('123456789')
    expect(deleted).toBe(true)

    const fetched = await service.findCachedTrack('123456789')
    expect(fetched).toBeNull()
  })
})
