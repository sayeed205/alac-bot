import { beforeEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'

import type { TelegramClient } from '@mtcute/bun'

import {
  type ActiveRipJob,
  type RipJobProgress,
  type RipJobSummary,
  RipOrchestrator,
} from '@/modules/alac/orchestrator/index.ts'
import { SequentialRipQueue } from '@/modules/alac/queue.ts'
import type { ITrackRipper } from '@/modules/alac/ripper.ts'
import type { IAlacService } from '@/modules/alac/service.ts'
import type { ISettingsService } from '@/modules/settings/service.ts'

describe('RipOrchestrator', () => {
  let orchestrator: RipOrchestrator
  let fakeTg: TelegramClient
  let mockService: IAlacService
  let mockQueue: SequentialRipQueue
  let mockRipper: ITrackRipper
  let mockSettingsService: ISettingsService

  beforeEach(() => {
    orchestrator = new RipOrchestrator()

    fakeTg = {
      sendCopy: mock(() => Promise.resolve({ id: 999 })),
      sendText: mock(() => Promise.resolve({ id: 123 })),
      sendMedia: mock(() =>
        Promise.resolve({
          id: 500,
          media: {
            type: 'audio',
            fileId: 'uploaded_file_id',
            uniqueFileId: 'uploaded_unique_id',
          },
        }),
      ),
      deleteMessagesById: mock(() => Promise.resolve()),
    } as unknown as TelegramClient

    mockService = {
      findCachedTrack: mock(() => Promise.resolve(null)),
      findTrackByFileUniqueId: mock(() => Promise.resolve(null)),
      findCachedTracks: mock(() => Promise.resolve(new Map())),
      saveTrack: mock(() => Promise.resolve({} as never)),
      deleteTrack: mock(() => Promise.resolve(true)),
      logRequest: mock(() => Promise.resolve({} as never)),
      getStats: mock(() => Promise.resolve({} as never)),
      searchCachedTracks: mock(() => Promise.resolve([])),
      getAllTrackIds: mock(() => Promise.resolve([])),
      deleteTracksNotIn: mock(() => Promise.resolve(0)),
    } as unknown as IAlacService

    mockQueue = new SequentialRipQueue()

    mockRipper = {
      rip: mock(async (_id, progressCb) => {
        progressCb?.('Downloading', 500, 1000)
        progressCb?.('Tagging ALAC metadata...', 1000, 1000)
        const tmpFile = '/tmp/test_orch_rip.m4a'
        fs.writeFileSync(tmpFile, 'dummy m4a content')
        return {
          filePath: tmpFile,
          codec: 'alac',
          bitDepth: 24,
          sampleRate: 48000,
          title: 'Orchestrator Track',
          artist: 'Orchestrator Artist',
          album: 'Orchestrator Album',
          duration: 180,
          genre: 'Pop',
          releaseDate: '2026-01-01',
          trackNumber: 1,
          trackCount: 1,
        }
      }),
    } as unknown as ITrackRipper

    mockSettingsService = {
      getSettings: mock(() => ({
        rippingMode: 'live',
        albumRipEnabled: true,
        playlistRipEnabled: true,
        artistRipEnabled: true,
        txtRipEnabled: true,
        multiLinkRipEnabled: true,
        maxCollectionTracks: 50,
        autoDumpEnabled: true,
        autoDumpStorefronts: ['us'],
      })),
      canServeCache: mock(() => true),
      canRipLive: mock(() => true),
      canRipAlbum: mock(() => true),
      canRipPlaylist: mock(() => true),
      canRipArtist: mock(() => true),
      canRipTxt: mock(() => true),
      canRipMultiLink: mock(() => true),
      getMaxCollectionTracks: mock(() => 50),
      isAutoDumpEnabled: mock(() => true),
      getAutoDumpStorefronts: mock(() => ['us']),
    } as unknown as ISettingsService

    orchestrator.setDependencies({
      tg: fakeTg,
      service: mockService,
      ripper: mockRipper,
      queue: mockQueue,
      settings: mockSettingsService,
      uploadRetryBaseMs: 10,
    })
  })

  it('manages jobs map and emits lifecycle events on startJob', async () => {
    const createdEvents: ActiveRipJob[] = []
    const progressEvents: RipJobProgress[] = []
    const completedEvents: RipJobSummary[] = []

    orchestrator.on('job:created', (job) => createdEvents.push(job))
    orchestrator.on('job:progress', (_job, prog) => progressEvents.push(prog))
    orchestrator.on('job:completed', (_job, summary) =>
      completedEvents.push(summary),
    )

    const summary = await orchestrator.startJob({
      chatId: 100,
      userId: 1,
      userName: 'TestUser',
      deliveryChatId: 100,
      isGroup: false,
      isForce: false,
      isCacheOnly: false,
      singleStorefront: 'us',
      parsedItems: [{ type: 'track', id: '12345' }],
      statusMsgId: 200,
      isAdmin: true,
    })

    expect(createdEvents.length).toBe(1)
    expect(createdEvents[0]?.id).toBeDefined()
    expect(progressEvents.length).toBeGreaterThan(0)
    expect(completedEvents.length).toBe(1)
    expect(summary?.totalTracks).toBe(1)
    expect(summary?.rippedCount).toBe(1)
    expect(summary?.cachedCount).toBe(0)
    expect(orchestrator.getActiveJobs().length).toBe(0) // Completed jobs are removed from active
  })

  it('delivers cached track immediately with empty caption and skips ripper', async () => {
    ;(
      mockService.findCachedTracks as ReturnType<typeof mock>
    ).mockResolvedValue(
      new Map([
        [
          '12345',
          {
            appleTrackId: '12345',
            messageId: 777,
            fileId: 'fid',
            fileUniqueId: 'fuid',
            title: 'Cached Title',
            artist: 'Cached Artist',
          },
        ],
      ]),
    )

    const summary = await orchestrator.startJob({
      chatId: 100,
      userId: 1,
      userName: 'TestUser',
      deliveryChatId: 100,
      isGroup: false,
      isForce: false,
      isCacheOnly: false,
      singleStorefront: 'us',
      parsedItems: [{ type: 'track', id: '12345' }],
      statusMsgId: 200,
      isAdmin: true,
    })

    expect(summary?.cachedCount).toBe(1)
    expect(summary?.rippedCount).toBe(0)
    expect(fakeTg.sendCopy).toHaveBeenCalledWith(
      expect.objectContaining({
        caption: { text: '' },
        message: 777,
      }),
    )
    expect(mockRipper.rip).not.toHaveBeenCalled()
  })

  it('cancels active job and emits job:cancelled event', async () => {
    let cancelledByResult: string | undefined
    let cancelledJob: ActiveRipJob | undefined

    orchestrator.on('job:cancelled', (job, by) => {
      cancelledJob = job
      cancelledByResult = by
    })

    // Create a job manually in orchestrator map
    const fakeJob: ActiveRipJob = {
      id: 'job_test_cancel',
      chatId: 100,
      userId: 1,
      jobHeader: 'Test Header',
      totalTracks: 1,
      statusMsgId: 123,
      controller: new AbortController(),
      isCancelled: false,
      cachedCount: 0,
      rippedCount: 0,
      failedCount: 0,
      completed: false,
      startTime: Date.now(),
    }
    orchestrator.activeJobs.set('job_test_cancel', fakeJob)

    const cancelled = orchestrator.cancelJob('job_test_cancel', 'AdminUser')
    expect(cancelled).toBe(true)
    expect(fakeJob.isCancelled).toBe(true)
    expect(fakeJob.cancelledBy).toBe('AdminUser')
    expect(fakeJob.controller.signal.aborted).toBe(true)
    expect(cancelledJob?.id).toBe('job_test_cancel')
    expect(cancelledByResult).toBe('AdminUser')
  })
})
