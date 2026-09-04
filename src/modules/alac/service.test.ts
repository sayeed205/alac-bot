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

  it('saves and retrieves a cached track with rich metadata', async () => {
    const saved = await service.saveTrack({
      appleTrackId: '123456789',
      messageId: 42,
      fileId: 'tg_file_id_test',
      fileUniqueId: 'tg_unique_id_test',
      title: 'Never Gonna Give You Up',
      artist: 'Rick Astley',
      album: 'Whenever You Need Somebody',
      duration: 215,
      bitDepth: 16,
      sampleRate: 44100,
    })

    expect(saved.appleTrackId).toBe('123456789')
    expect(saved.messageId).toBe(42)
    expect(saved.title).toBe('Never Gonna Give You Up')
    expect(saved.artist).toBe('Rick Astley')
    expect(saved.bitDepth).toBe(16)

    const fetched = await service.findCachedTrack('123456789')
    expect(fetched).not.toBeNull()
    expect(fetched?.messageId).toBe(42)
    expect(fetched?.title).toBe('Never Gonna Give You Up')
    expect(fetched?.artist).toBe('Rick Astley')
  })

  it('upserts an existing track when saved with updated details', async () => {
    await service.saveTrack({
      appleTrackId: '123456789',
      messageId: 100,
      fileId: 'tg_file_id_updated',
      title: 'Never Gonna Give You Up (Remastered)',
    })

    const fetched = await service.findCachedTrack('123456789')
    expect(fetched?.messageId).toBe(100)
    expect(fetched?.title).toBe('Never Gonna Give You Up (Remastered)')
  })

  it('searches cached tracks by title, artist, or track ID', async () => {
    // Empty search query returns empty array
    expect(await service.searchCachedTracks('')).toEqual([])
    expect(await service.searchCachedTracks('   ')).toEqual([])

    // Save another track for search diversity
    await service.saveTrack({
      appleTrackId: '555666777',
      messageId: 200,
      fileId: 'tg_file_id_3',
      title: 'Together Forever',
      artist: 'Rick Astley',
      album: 'Whenever You Need Somebody',
      duration: 205,
    })

    // Search by title substring (case-insensitive)
    const titleResults = await service.searchCachedTracks('give you up')
    expect(titleResults.length).toBe(1)
    expect(titleResults[0]?.appleTrackId).toBe('123456789')

    // Search by artist substring matches multiple tracks
    const artistResults = await service.searchCachedTracks('astley')
    expect(artistResults.length).toBe(2)

    // Search by direct apple track ID
    const idResults = await service.searchCachedTracks('555666777')
    expect(idResults.length).toBe(1)
    expect(idResults[0]?.title).toBe('Together Forever')

    // Search with no matches
    const noResults = await service.searchCachedTracks('NonExistentArtistXYZ')
    expect(noResults.length).toBe(0)
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

  it('calculates aggregated statistics with getStats', async () => {
    await service.logRequest({
      telegramId: 6252490183,
      chatId: -100123456,
      appleTrackId: '123456789',
      isCacheHit: false,
      durationMs: 15000,
      status: 'completed',
    })

    await service.logRequest({
      telegramId: 6252490183,
      chatId: -100123456,
      appleTrackId: '987654321',
      isCacheHit: false,
      durationMs: 5000,
      status: 'failed',
      errorReason: 'Stream timed out',
    })

    const stats = await service.getStats()
    expect(stats.totalRequests).toBeGreaterThanOrEqual(3)
    expect(stats.cacheHits).toBeGreaterThanOrEqual(1)
    expect(stats.cacheMisses).toBeGreaterThanOrEqual(2)
    expect(stats.totalFailedRequests).toBeGreaterThanOrEqual(1)
    expect(stats.topTracks.length).toBeGreaterThanOrEqual(1)
    expect(stats.topTracks[0]?.appleTrackId).toBe('123456789')
  })

  it('deletes an existing track', async () => {
    await service.saveTrack({
      appleTrackId: 'test_delete_id',
      messageId: 300,
      fileId: 'tg_file_id_del',
    })

    const deleted = await service.deleteTrack('test_delete_id')
    expect(deleted).toBe(true)

    const fetched = await service.findCachedTrack('test_delete_id')
    expect(fetched).toBeNull()
  })
})
