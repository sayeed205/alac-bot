import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'bun:test'

import type { AppDatabase } from '@/db/index.ts'
import { AlacService, type SaveTrackInput } from '@/modules/alac/service.ts'

import { setupTestDb } from '../../test-db.ts'

const makeTrackInput = (
  overrides: Partial<SaveTrackInput> & {
    appleTrackId: string
    messageId: number
    fileId: string
  },
): SaveTrackInput => ({
  fileUniqueId: `uniq_${overrides.appleTrackId}`,
  title: 'Test Title',
  artist: 'Rick Astley',
  album: 'Whenever You Need Somebody',
  duration: 200,
  bitDepth: 16,
  sampleRate: 44100,
  genre: 'Pop',
  releaseDate: '1987-11-12',
  trackNumber: 1,
  trackCount: 10,
  ...overrides,
})

describe('AlacService', () => {
  let db: AppDatabase
  let cleanDb: () => Promise<void>
  let close: () => Promise<void>
  let service: AlacService

  beforeAll(async () => {
    const testEnv = await setupTestDb()
    db = testEnv.db
    cleanDb = testEnv.cleanDb
    close = testEnv.close
    service = new AlacService(db)
  })

  beforeEach(async () => {
    await cleanDb()
  })

  afterAll(async () => {
    await close()
  })

  it('returns null when track is not cached', async () => {
    const track = await service.findCachedTrack('non_existent_id')
    expect(track).toBeNull()
  })

  it('saves and retrieves a cached track with rich metadata', async () => {
    const input = makeTrackInput({
      appleTrackId: '123456789',
      messageId: 100,
      fileId: 'file_123',
    })

    const saved = await service.saveTrack(input)
    expect(saved.appleTrackId).toBe('123456789')
    expect(saved.messageId).toBe(100)
    expect(saved.title).toBe('Test Title')
    expect(saved.artist).toBe('Rick Astley')

    const found = await service.findCachedTrack('123456789')
    expect(found).not.toBeNull()
    expect(found?.appleTrackId).toBe('123456789')
    expect(found?.messageId).toBe(100)
    expect(found?.bitDepth).toBe(16)
    expect(found?.sampleRate).toBe(44100)
  })

  it('upserts an existing track when saved with updated details', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: '123456789',
        messageId: 100,
        fileId: 'file_old',
      }),
    )

    const updated = await service.saveTrack({
      ...makeTrackInput({
        appleTrackId: '123456789',
        messageId: 200,
        fileId: 'file_new',
      }),
      title: 'Updated Title',
    })

    expect(updated.messageId).toBe(200)
    expect(updated.fileId).toBe('file_new')
    expect(updated.title).toBe('Updated Title')

    const found = await service.findCachedTrack('123456789')
    expect(found?.messageId).toBe(200)
    expect(found?.title).toBe('Updated Title')
  })

  it('searches cached tracks with typo-tolerance and cross-field matching', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: '123456789',
        messageId: 100,
        fileId: 'file_123',
        title: 'Never Gonna Give You Up',
        artist: 'Rick Astley',
        album: 'Whenever You Need Somebody',
      }),
    )

    await service.saveTrack(
      makeTrackInput({
        appleTrackId: '987654321',
        messageId: 101,
        fileId: 'file_987',
        title: 'Together Forever',
        artist: 'Rick Astley',
        album: 'Whenever You Need Somebody',
      }),
    )

    await service.saveTrack(
      makeTrackInput({
        appleTrackId: '555666777',
        messageId: 102,
        fileId: 'file_555',
        title: 'Starboy',
        artist: 'The Weeknd',
        album: 'Starboy',
      }),
    )

    // Exact title substring
    const titleResults = await service.searchCachedTracks('give you up')
    expect(titleResults.length).toBe(1)
    expect(titleResults[0]?.appleTrackId).toBe('123456789')

    // Exact artist substring
    const artistResults = await service.searchCachedTracks('astley')
    expect(artistResults.length).toBe(2)

    // Exact track ID match
    const idResults = await service.searchCachedTracks('555666777')
    expect(idResults.length).toBe(1)
    expect(idResults[0]?.appleTrackId).toBe('555666777')

    // Typo match: 'rik' matching 'Rick'
    const typoResults = await service.searchCachedTracks('rik')
    expect(typoResults.length).toBe(2)

    // Cross-field match with typo: 'never gonna rik'
    const crossFieldResults =
      await service.searchCachedTracks('never gonna rik')
    expect(crossFieldResults.length).toBe(1)
    expect(crossFieldResults[0]?.appleTrackId).toBe('123456789')

    // Non-existent search
    const noResults = await service.searchCachedTracks(
      'NonExistentArtistXYZ12345',
    )
    expect(noResults.length).toBe(0)
  })

  it('batch retrieves multiple cached tracks with findCachedTracks', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'track_1',
        messageId: 1,
        fileId: 'f1',
      }),
    )
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'track_2',
        messageId: 2,
        fileId: 'f2',
      }),
    )

    const map = await service.findCachedTracks([
      'track_1',
      'track_2',
      'track_missing',
    ])
    expect(map.size).toBe(2)
    expect(map.get('track_1')?.appleTrackId).toBe('track_1')
    expect(map.get('track_2')?.appleTrackId).toBe('track_2')
    expect(map.has('track_missing')).toBe(false)
  })

  it('logs a request entry in the analytics requests table', async () => {
    await service.logRequest({
      telegramId: 123456,
      chatId: -100123456789,
      appleTrackId: '123456789',
      isCacheHit: false,
      durationMs: 4500,
      status: 'completed',
    })

    const stats = await service.getStats()
    expect(stats.totalRequests).toBe(1)
    expect(stats.cacheHits).toBe(0)
    expect(stats.cacheMisses).toBe(1)
  })

  it('calculates aggregated statistics with getStats', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'track_a',
        messageId: 10,
        fileId: 'f_a',
      }),
    )
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'track_b',
        messageId: 20,
        fileId: 'f_b',
      }),
    )

    await service.logRequest({
      telegramId: 1,
      chatId: 10,
      appleTrackId: 'track_a',
      isCacheHit: true,
      durationMs: 50,
      status: 'completed',
    })
    await service.logRequest({
      telegramId: 2,
      chatId: 20,
      appleTrackId: 'track_a',
      isCacheHit: false,
      durationMs: 5000,
      status: 'completed',
    })
    await service.logRequest({
      telegramId: 3,
      chatId: 30,
      appleTrackId: 'track_b',
      isCacheHit: false,
      durationMs: 3000,
      status: 'failed',
    })

    const stats = await service.getStats()
    expect(stats.totalCachedTracks).toBe(2)
    expect(stats.totalRequests).toBe(3)
    expect(stats.cacheHits).toBe(1)
    expect(stats.cacheMisses).toBe(2)
    expect(stats.totalFailedRequests).toBe(1)
    expect(stats.avgCacheDurationMs).toBe(50)
    expect(stats.avgRipDurationMs).toBe(5000)
    expect(stats.topTracks.length).toBe(1)
    expect(stats.topTracks[0]?.appleTrackId).toBe('track_a')
  })

  it('deletes an existing track', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'track_to_delete',
        messageId: 1,
        fileId: 'f1',
      }),
    )

    const deleted = await service.deleteTrack('track_to_delete')
    expect(deleted).toBe(true)

    const found = await service.findCachedTrack('track_to_delete')
    expect(found).toBeNull()

    const notDeleted = await service.deleteTrack('non_existent')
    expect(notDeleted).toBe(false)
  })

  it('gets all track ids and deletes tracks not in list', async () => {
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'keep_1',
        messageId: 1,
        fileId: 'f1',
      }),
    )
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'keep_2',
        messageId: 2,
        fileId: 'f2',
      }),
    )
    await service.saveTrack(
      makeTrackInput({
        appleTrackId: 'remove_1',
        messageId: 3,
        fileId: 'f3',
      }),
    )

    const allIds = await service.getAllTrackIds()
    expect(allIds.length).toBe(3)

    const removedCount = await service.deleteTracksNotIn(['keep_1', 'keep_2'])
    expect(removedCount).toBe(1)

    const remainingIds = await service.getAllTrackIds()
    expect(remainingIds.length).toBe(2)
    expect(remainingIds).toContain('keep_1')
    expect(remainingIds).toContain('keep_2')
    expect(remainingIds).not.toContain('remove_1')
  })
})
