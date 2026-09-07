import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'bun:test'

import type { AppDatabase } from '@/db/index.ts'
import {
  RequestLogRepository,
  type SaveTrackInput,
  StatsRepository,
  TrackRepository,
} from '@/modules/alac/repositories/index.ts'

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

describe('Repositories (TrackRepository, RequestLogRepository, StatsRepository)', () => {
  let db: AppDatabase
  let cleanDb: () => Promise<void>
  let close: () => Promise<void>
  let tracks: TrackRepository
  let requestLogs: RequestLogRepository
  let stats: StatsRepository

  beforeAll(async () => {
    const testEnv = await setupTestDb()
    db = testEnv.db
    cleanDb = testEnv.cleanDb
    close = testEnv.close
    tracks = new TrackRepository(db)
    requestLogs = new RequestLogRepository(db)
    stats = new StatsRepository(db)
  })

  beforeEach(async () => {
    await cleanDb()
  })

  afterAll(async () => {
    await close()
  })

  it('TrackRepository saves, finds, and deletes tracks', async () => {
    const track = await tracks.saveTrack(
      makeTrackInput({
        appleTrackId: '1001',
        messageId: 50,
        fileId: 'file_1001',
      }),
    )
    expect(track.appleTrackId).toBe('1001')

    const found = await tracks.findCachedTrack('1001')
    expect(found).not.toBeNull()
    expect(found?.messageId).toBe(50)

    const byUnique = await tracks.findTrackByFileUniqueId('uniq_1001')
    expect(byUnique?.appleTrackId).toBe('1001')

    const deleted = await tracks.deleteTrack('1001')
    expect(deleted).toBe(true)
    expect(await tracks.findCachedTrack('1001')).toBeNull()
  })

  it('RequestLogRepository logs user rip requests', async () => {
    await requestLogs.logRequest({
      telegramId: 42,
      chatId: 42,
      appleTrackId: '1001',
      isCacheHit: false,
      durationMs: 1200,
      status: 'completed',
    })

    const currentStats = await stats.getStats()
    expect(currentStats.totalRequests).toBe(1)
    expect(currentStats.cacheHits).toBe(0)
    expect(currentStats.avgRipDurationMs).toBe(1200)
  })

  it('StatsRepository calculates hit ratio and top tracks correctly', async () => {
    await tracks.saveTrack(
      makeTrackInput({
        appleTrackId: '999',
        messageId: 10,
        fileId: 'f_999',
      }),
    )

    await requestLogs.logRequest({
      telegramId: 1,
      chatId: 1,
      appleTrackId: '999',
      isCacheHit: true,
      durationMs: 50,
      status: 'completed',
    })

    await requestLogs.logRequest({
      telegramId: 1,
      chatId: 1,
      appleTrackId: '999',
      isCacheHit: false,
      durationMs: 2000,
      status: 'completed',
    })

    const data = await stats.getStats()
    expect(data.totalCachedTracks).toBe(1)
    expect(data.totalRequests).toBe(2)
    expect(data.cacheHits).toBe(1)
    expect(data.cacheHitRatio).toBe(50)
    expect(data.topTracks.length).toBe(1)
    expect(data.topTracks[0]?.appleTrackId).toBe('999')
  })
})
