import { EventEmitter } from 'node:events'
import { existsSync, rmSync, unlinkSync } from 'node:fs'
import { mkdir } from 'node:fs/promises'
import os from 'node:os'
import { join } from 'node:path'

import { html, type Message } from '@mtcute/bun'

import { env } from '@/env.ts'
import { catalogService } from '@/modules/alac/catalog/index.ts'
import { sendDumpCopy } from '@/modules/alac/delivery.ts'
import { formatDumpCaption } from '@/modules/alac/indexer.ts'
import { fetchPlaylistTracks } from '@/modules/alac/playlist.ts'
import { abortableSleep, type TrackRipResult } from '@/modules/alac/ripper.ts'
import { settingsService as defaultSettingsService } from '@/modules/settings/service.ts'
import { BoundedChannel } from '@/utils/channel.ts'
import { debug, debugSpan, error, info, warn } from '@/utils/logger.ts'
import { formatByteProgress } from '@/utils/progress.ts'

import type {
  ActiveRipJob,
  OrchestratorDependencies,
  RipJobOptions,
  RipJobProgress,
  RipJobSummary,
} from './types.ts'

interface ResolvedTrackItem {
  id: string
  title?: string
  artist?: string
  storefront?: string
}

interface PipelineItem {
  trackId: string
  job: ActiveRipJob
  storefront?: string
  meta?: { title?: string; artist?: string }
}

interface PipelineRipResult {
  trackId: string
  job: ActiveRipJob
  ripResult: TrackRipResult
  startTime: number
}

export class RipOrchestrator extends EventEmitter {
  public readonly activeJobs = new Map<string, ActiveRipJob>()
  private readonly jobs = this.activeJobs
  private deps: OrchestratorDependencies | null = null

  public setDependencies(deps: OrchestratorDependencies): void {
    this.deps = deps
  }

  public getActiveJobs(): ActiveRipJob[] {
    return Array.from(this.jobs.values()).filter((j) => !j.completed)
  }

  public getJob(id: string): ActiveRipJob | undefined {
    return this.jobs.get(id)
  }

  public cancelJob(id: string, cancelledBy?: string): boolean {
    const job = this.jobs.get(id)
    if (!job || job.isCancelled || job.completed) {
      return false
    }

    job.isCancelled = true
    job.cancelledBy = cancelledBy
    job.controller.abort()
    this.jobs.delete(id)
    this.emit('job:cancelled', job, cancelledBy)
    return true
  }

  public async startJob(
    options: RipJobOptions,
    depsOverride?: OrchestratorDependencies,
  ): Promise<RipJobSummary | null> {
    const deps = depsOverride || this.deps
    if (!deps) {
      throw new Error(
        'RipOrchestrator dependencies not configured. Call setDependencies() first.',
      )
    }

    const {
      chatId,
      userId,
      userName,
      deliveryChatId,
      isGroup,
      isForce,
      isCacheOnly,
      singleStorefront,
      parsedItems,
      statusMsgId,
      replyToMessageId,
      isAdmin,
    } = options

    const {
      tg,
      service,
      ripper,
      queue,
      settings: customSettings,
      uploadRetryBaseMs = 3000,
    } = deps

    const settings = customSettings || defaultSettingsService

    // Fast maintenance check
    if (!settings.canRipLive(isAdmin) && !isCacheOnly) {
      throw new Error(
        'Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now.',
      )
    }

    const jobController = new AbortController()
    const jobId = `job_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`

    let jobHeader = 'Apple Music Lossless Rip'
    if (parsedItems.length === 1 && parsedItems[0]) {
      const it = parsedItems[0]
      if (it.type === 'album') jobHeader = `Album ${it.id}`
      else if (it.type === 'playlist') jobHeader = `Playlist ${it.id}`
      else if (it.type === 'artist') jobHeader = `Artist ${it.id}`
      else if (it.type === 'track') jobHeader = `Track ${it.id}`
    } else if (parsedItems.length > 1) {
      jobHeader = `Batch (${parsedItems.length} links)`
    }

    const currentJob: ActiveRipJob = {
      id: jobId,
      chatId,
      userId,
      userName,
      jobHeader,
      totalTracks: 0,
      statusMsgId,
      controller: jobController,
      isCancelled: false,
      cachedCount: 0,
      rippedCount: 0,
      failedCount: 0,
      completed: false,
      startTime: Date.now(),
    }

    this.jobs.set(jobId, currentJob)
    this.emit('job:created', currentJob)

    const emitProgress = (
      activityOverride?: string,
      activeDownloadText?: string,
      activeUploadText?: string,
    ) => {
      const completedTracks =
        currentJob.cachedCount + currentJob.rippedCount + currentJob.failedCount
      const percent =
        currentJob.totalTracks > 0
          ? Math.round((completedTracks / currentJob.totalTracks) * 100)
          : 0

      const prog: RipJobProgress = {
        jobId: currentJob.id,
        totalTracks: currentJob.totalTracks,
        completedTracks,
        cachedCount: currentJob.cachedCount,
        rippedCount: currentJob.rippedCount,
        failedCount: currentJob.failedCount,
        skippedCount: 0,
        percent,
        activeDownloadText,
        activeUploadText,
        activityOverride,
      }
      this.emit('job:progress', currentJob, prog)
    }

    try {
      emitProgress('Resolving metadata & tracklist...')

      const resolvedTracks: ResolvedTrackItem[] = []
      let albumName: string | undefined
      let albumArtist: string | undefined

      for (const item of parsedItems) {
        if (jobController.signal.aborted) {
          throw new Error('Download was cancelled')
        }

        const effectiveSf = item.storefront || singleStorefront || 'us'

        try {
          if (item.type === 'track') {
            resolvedTracks.push({ id: item.id, storefront: effectiveSf })
          } else if (item.type === 'album') {
            const albumData = await catalogService.fetchAlbumTracks(
              item.id,
              effectiveSf,
            )
            albumName = albumData.album.album
            albumArtist = albumData.album.artist
            for (const t of albumData.tracks) {
              resolvedTracks.push({
                id: t.id,
                title: t.title,
                artist: t.artist,
                storefront: effectiveSf,
              })
            }
          } else if (item.type === 'artist') {
            const artistData = await catalogService.fetchArtistTracks(
              item.id,
              effectiveSf,
            )
            albumArtist = artistData.artistName
            for (const t of artistData.tracks) {
              resolvedTracks.push({
                id: t.id,
                title: t.title,
                artist: t.artist,
                storefront: effectiveSf,
              })
            }
          } else if (item.type === 'playlist') {
            const playlistData = await fetchPlaylistTracks(item.id, effectiveSf)
            for (const t of playlistData.tracks) {
              resolvedTracks.push({
                id: t.id,
                title: t.title,
                artist: t.artist,
                storefront: effectiveSf,
              })
            }
          }
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err)
          error('Failed to resolve target item', {
            type: item.type,
            id: item.id,
            error: errMsg,
          })
          throw new Error(`${item.type} ${item.id}: ${errMsg}`)
        }
      }

      if (resolvedTracks.length === 0) {
        throw new Error('No valid tracks found to process.')
      }

      // Deduplicate track IDs while preserving order
      const seenTrackIds = new Set<string>()
      const uniqueTracks: ResolvedTrackItem[] = []
      for (const t of resolvedTracks) {
        if (!seenTrackIds.has(t.id)) {
          seenTrackIds.add(t.id)
          uniqueTracks.push(t)
        }
      }

      // Cap collections for non-admins if configured
      let cappedCount = 0
      const maxCollectionLimit = settings.getMaxCollectionTracks()
      let tracksToProcess = uniqueTracks
      if (
        !isAdmin &&
        maxCollectionLimit > 0 &&
        uniqueTracks.length > maxCollectionLimit
      ) {
        cappedCount = uniqueTracks.length - maxCollectionLimit
        tracksToProcess = uniqueTracks.slice(0, maxCollectionLimit)
        warn('Collection capped for non-admin user', {
          userId,
          original: uniqueTracks.length,
          capped: maxCollectionLimit,
        })
      }

      // Determine clean job header
      if (albumName && albumArtist) {
        currentJob.jobHeader = `${albumArtist} - ${albumName}`
      } else if (tracksToProcess.length === 1 && tracksToProcess[0]) {
        const first = tracksToProcess[0]
        if (first.title && first.artist) {
          currentJob.jobHeader = `${first.artist} - ${first.title}`
        } else {
          currentJob.jobHeader = `Track ${first.id}`
        }
      } else {
        currentJob.jobHeader = `Batch (${tracksToProcess.length} tracks)`
      }

      currentJob.totalTracks = tracksToProcess.length

      // Check database cache for tracks
      const requestedIds = tracksToProcess.map((t) => t.id)
      const existingTracksMap = await service.findCachedTracks(requestedIds)

      // Purge cache if force re-rip (-f)
      if (isForce && isAdmin) {
        const oldMessageIds: number[] = []
        for (const item of tracksToProcess) {
          const cached = existingTracksMap.get(item.id)
          if (cached) {
            oldMessageIds.push(cached.messageId)
            existingTracksMap.delete(item.id)
            await service.deleteTrack(item.id).catch(() => null)
          }
        }
        if (oldMessageIds.length > 0) {
          debug('Deleting old dump messages on force re-rip prior to queue', {
            count: oldMessageIds.length,
            old_message_ids: oldMessageIds,
          })
          await tg
            .deleteMessagesById(env.DUMP_CHANNEL_ID, oldMessageIds)
            .catch(() => null)
        }
      }

      // Pre-queue cache handling: deliver cached items immediately to DM
      const uncachedItems: ResolvedTrackItem[] = []
      let cachedCount = 0
      const isMultiTrack = tracksToProcess.length > 1

      for (const item of tracksToProcess) {
        if (jobController.signal.aborted) {
          throw new Error('Download was cancelled')
        }

        const cached = existingTracksMap.get(item.id)
        if (!cached) {
          uncachedItems.push(item)
          continue
        }

        if (isCacheOnly) {
          cachedCount++
          currentJob.cachedCount = cachedCount
          info('Track already cached in dump channel', { track_id: item.id })
          emitProgress('Recognized cached tracks...')
        } else {
          try {
            await sendDumpCopy({
              tg,
              toChatId: deliveryChatId,
              messageId: cached.messageId,
              replyTo: deliveryChatId === chatId ? replyToMessageId : undefined,
              silent: isMultiTrack,
            })

            info('Cache hit: delivered', {
              track_id: item.id,
              time: '0ms',
            })

            await service.logRequest({
              telegramId: userId,
              chatId: chatId,
              appleTrackId: item.id,
              isCacheHit: true,
              durationMs: 0,
              status: 'completed',
            })

            cachedCount++
            currentJob.cachedCount = cachedCount
            emitProgress('Delivered cached tracks...')
          } catch (err: unknown) {
            error('Failed to deliver cached track copy, marking for re-rip', {
              track_id: item.id,
              error: String(err),
            })
            uncachedItems.push(item)
          }
        }
      }

      // If all tracks were satisfied from cache, we're done!
      if (uncachedItems.length === 0) {
        currentJob.completed = true
        const summary: RipJobSummary = {
          jobId,
          jobHeader: currentJob.jobHeader,
          totalTracks: currentJob.totalTracks,
          cachedCount,
          rippedCount: 0,
          failedCount: 0,
          failedTracks: [],
          skippedUncachedTracks: [],
          totalElapsedSec: '0.0',
          cappedCount,
          maxCollectionLimit,
          isCacheOnly,
          isGroup,
        }
        this.emit('job:completed', currentJob, summary)
        return summary
      }

      // If in cache-only mode (non-admin) and there are uncached items
      if (
        !settings.canRipLive(isAdmin) &&
        !isCacheOnly &&
        uncachedItems.length > 0
      ) {
        currentJob.completed = true
        const summary: RipJobSummary = {
          jobId,
          jobHeader: currentJob.jobHeader,
          totalTracks: currentJob.totalTracks,
          cachedCount,
          rippedCount: 0,
          failedCount: 0,
          failedTracks: [],
          skippedUncachedTracks: uncachedItems.map((i) => i.id),
          totalElapsedSec: '0.0',
          cappedCount,
          maxCollectionLimit,
          isCacheOnly,
          isGroup,
        }
        this.emit('job:completed', currentJob, summary)
        return summary
      }

      // Enqueue uncached items for ripping in sequential queue
      emitProgress('Queued for ripping...')

      const failedTracks: Array<{ id: string; error: string }> = []
      let totalElapsedSec = '0.0'

      info('Rip job queued', {
        jobId,
        tracksCount: uncachedItems.length,
        force: isForce,
        isGroup,
        isCacheOnly,
        deliveryChatId,
      })

      const queueStartTime = Date.now()

      const summary = await queue.enqueue(async () => {
        debug('Rip job started from queue', { jobId })
        this.emit('job:started', currentJob)

        const ripJobDir = join(
          os.tmpdir(),
          `rip_job_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`,
        )
        await mkdir(ripJobDir, { recursive: true }).catch(() => null)

        let rippedCountInner = 0
        let activeDownloadText = ''
        let activeUploadText = ''

        const downloadChannel = new BoundedChannel<PipelineItem>(1)
        const uploadChannel = new BoundedChannel<PipelineRipResult>(2)

        // Producer: Feeds uncached tracks into download channel
        const producerPromise = (async () => {
          try {
            for (const item of uncachedItems) {
              if (currentJob.isCancelled || jobController.signal.aborted) {
                break
              }
              await downloadChannel.push({
                trackId: item.id,
                job: currentJob,
                storefront: item.storefront,
                meta: { title: item.title, artist: item.artist },
              })
            }
          } finally {
            downloadChannel.close()
          }
        })()

        // Worker 1: Downloader
        const downloadWorkerPromise = (async () => {
          try {
            for await (const item of downloadChannel) {
              if (currentJob.isCancelled || jobController.signal.aborted) {
                break
              }

              const { trackId, storefront, meta } = item
              const trackStartTime = Date.now()
              using _span = debugSpan('track_job', {
                track_id: trackId,
              }).enter()

              const trackLabel =
                meta?.title && meta?.artist
                  ? `${meta.artist} - ${meta.title}`
                  : `Track ${trackId}`

              const onProgress = (
                status: string,
                downloaded?: number,
                total?: number,
              ) => {
                if (downloaded !== undefined && total !== undefined) {
                  const prog = formatByteProgress(downloaded, total)
                  activeDownloadText = `⬇️ <b>${html.escape(trackLabel)}:</b> <code>${prog}</code>`
                } else {
                  activeDownloadText = `⬇️ <b>${html.escape(trackLabel)}:</b> ${html.escape(status)}`
                }
                currentJob.activeActionText = activeDownloadText
                emitProgress(undefined, activeDownloadText, activeUploadText)
              }

              try {
                const ripResult = await ripper.rip(
                  trackId,
                  onProgress,
                  storefront,
                  jobController.signal,
                  ripJobDir,
                )

                activeDownloadText = ''
                currentJob.activeActionText = ''

                await uploadChannel.push({
                  trackId,
                  job: currentJob,
                  ripResult,
                  startTime: trackStartTime,
                })
              } catch (err: unknown) {
                activeDownloadText = ''
                currentJob.activeActionText = ''

                if (currentJob.isCancelled || jobController.signal.aborted) {
                  break
                }

                const errMsg = err instanceof Error ? err.message : String(err)
                failedTracks.push({ id: trackId, error: errMsg })
                currentJob.failedCount = failedTracks.length

                error('Rip job failed', {
                  track_id: trackId,
                  duration_ms: Date.now() - trackStartTime,
                  error: errMsg,
                })

                await service.logRequest({
                  telegramId: userId,
                  chatId: chatId,
                  appleTrackId: trackId,
                  isCacheHit: false,
                  durationMs: Date.now() - trackStartTime,
                  status: 'failed',
                  errorReason: errMsg,
                })

                emitProgress('Processing next track...')

                // Circuit breaker: abort remaining batch if mirror server is offline/timing out
                if (
                  errMsg.toLowerCase().includes('timed out') ||
                  errMsg.toLowerCase().includes('mirror') ||
                  errMsg.toLowerCase().includes('502') ||
                  errMsg.toLowerCase().includes('503')
                ) {
                  failedTracks.push({
                    id: 'Remaining tracks',
                    error:
                      'Mirror service offline / unreachable (stopped remaining batch)',
                  })
                  error(
                    'Lossless mirror appears to be down, aborting remaining batch to prevent repeated timeouts',
                    { track_id: trackId, error: errMsg },
                  )
                  break
                }
              }
            }
          } finally {
            uploadChannel.close()
          }
        })()

        // Worker 2: Uploader
        const uploadWorkerPromise = (async () => {
          try {
            for await (const uploadItem of uploadChannel) {
              if (currentJob.isCancelled || jobController.signal.aborted) {
                if (existsSync(uploadItem.ripResult.filePath)) {
                  unlinkSync(uploadItem.ripResult.filePath)
                }
                continue
              }

              const { trackId, ripResult, startTime } = uploadItem
              const trackLabel = `${ripResult.artist} - ${ripResult.title}`

              using _span = debugSpan('track_job', {
                track_id: trackId,
              }).enter()
              debug('Uploading track to dump channel', {
                track_id: trackId,
                file: ripResult.filePath,
              })

              const caption = formatDumpCaption({
                title: ripResult.title,
                artist: ripResult.artist,
                album: ripResult.album,
                appleTrackId: trackId,
                bitDepth: ripResult.bitDepth,
                sampleRate: ripResult.sampleRate,
                duration: ripResult.duration,
                codec: ripResult.codec,
                genre: ripResult.genre,
                releaseDate: ripResult.releaseDate,
                trackNumber: ripResult.trackNumber,
                trackCount: ripResult.trackCount,
              })

              activeUploadText = `⬆️ <b>Uploading:</b> <i>${html.escape(trackLabel)}</i>`
              currentJob.activeActionText = activeUploadText
              emitProgress(undefined, activeDownloadText, activeUploadText)

              let dumpMsg: Message | null = null
              const maxRetries = 4

              for (let attempt = 1; attempt <= maxRetries; attempt++) {
                try {
                  dumpMsg = await tg.sendMedia(
                    env.DUMP_CHANNEL_ID,
                    {
                      type: 'audio',
                      file: ripResult.filePath,
                      title: ripResult.title,
                      performer: ripResult.artist,
                      duration: ripResult.duration,
                    },
                    {
                      caption,
                      progressCallback: (uploaded, total) => {
                        if (uploaded !== undefined && total !== undefined) {
                          const prog = formatByteProgress(uploaded, total)
                          activeUploadText = `⬆️ <b>Uploading:</b> <code>${prog}</code>`
                          currentJob.activeActionText = activeUploadText
                          emitProgress(
                            undefined,
                            activeDownloadText,
                            activeUploadText,
                          )
                        }
                      },
                    },
                  )
                  break
                } catch (uploadErr: unknown) {
                  if (currentJob.isCancelled || jobController.signal.aborted) {
                    break
                  }
                  if (attempt < maxRetries) {
                    const delay =
                      uploadRetryBaseMs * 2 ** (attempt - 1) +
                      Math.random() * 500
                    warn('Track upload to dump failed, retrying', {
                      track_id: trackId,
                      attempt,
                      max_retries: maxRetries,
                      delay_ms: Math.round(delay),
                      error:
                        uploadErr instanceof Error
                          ? uploadErr.message
                          : String(uploadErr),
                    })
                    await abortableSleep(delay, jobController.signal).catch(
                      () => null,
                    )
                  } else {
                    error('All upload retries exhausted for track', {
                      track_id: trackId,
                      attempts: maxRetries,
                      error:
                        uploadErr instanceof Error
                          ? uploadErr.message
                          : String(uploadErr),
                    })
                    throw uploadErr
                  }
                }
              }

              if (dumpMsg?.media?.type === 'audio') {
                try {
                  await service.saveTrack({
                    appleTrackId: trackId,
                    messageId: dumpMsg.id,
                    fileId: dumpMsg.media.fileId,
                    fileUniqueId: dumpMsg.media.uniqueFileId,
                    title: ripResult.title,
                    artist: ripResult.artist,
                    album: ripResult.album,
                    duration: ripResult.duration,
                    bitDepth: ripResult.bitDepth,
                    sampleRate: ripResult.sampleRate,
                    genre: ripResult.genre,
                    releaseDate: ripResult.releaseDate,
                    trackNumber: ripResult.trackNumber,
                    trackCount: ripResult.trackCount,
                  })

                  if (!isCacheOnly) {
                    await sendDumpCopy({
                      tg,
                      toChatId: deliveryChatId,
                      messageId: dumpMsg.id,
                      replyTo:
                        deliveryChatId === chatId
                          ? replyToMessageId
                          : undefined,
                      silent: isMultiTrack,
                    })
                  }

                  activeUploadText = ''
                  currentJob.activeActionText = ''
                  rippedCountInner++
                  currentJob.rippedCount = rippedCountInner
                  const totalDurationMs = Date.now() - startTime
                  info(
                    isCacheOnly ? 'Track cached to dump' : 'Track completed',
                    {
                      track: `${ripResult.artist} - ${ripResult.title}`,
                      time: `${(totalDurationMs / 1000).toFixed(1)}s`,
                    },
                  )

                  await service.logRequest({
                    telegramId: userId,
                    chatId: chatId,
                    appleTrackId: trackId,
                    isCacheHit: false,
                    durationMs: totalDurationMs,
                    status: 'completed',
                  })
                } catch (err: unknown) {
                  activeUploadText = ''
                  currentJob.activeActionText = ''
                  if (currentJob.isCancelled || jobController.signal.aborted) {
                    break
                  }
                  const errMsg =
                    err instanceof Error ? err.message : String(err)
                  failedTracks.push({ id: trackId, error: errMsg })
                  currentJob.failedCount = failedTracks.length
                  error('Track upload failed', {
                    track_id: trackId,
                    error: errMsg,
                  })
                  await service.logRequest({
                    telegramId: userId,
                    chatId: chatId,
                    appleTrackId: trackId,
                    isCacheHit: false,
                    durationMs: Date.now() - startTime,
                    status: 'failed',
                    errorReason: errMsg,
                  })
                }
              } else if (
                !currentJob.isCancelled &&
                !jobController.signal.aborted
              ) {
                const errMsg = 'Upload failed: no audio media returned'
                failedTracks.push({ id: trackId, error: errMsg })
                currentJob.failedCount = failedTracks.length
                error('Track upload failed: no audio media returned', {
                  track_id: trackId,
                })
              }

              if (existsSync(ripResult.filePath)) {
                try {
                  unlinkSync(ripResult.filePath)
                } catch {}
              }
            }
          } finally {
            if (existsSync(ripJobDir)) {
              try {
                rmSync(ripJobDir, { recursive: true, force: true })
              } catch {}
            }
          }
        })()

        await Promise.all([
          producerPromise,
          downloadWorkerPromise,
          uploadWorkerPromise,
        ])

        totalElapsedSec = ((Date.now() - queueStartTime) / 1000).toFixed(1)

        const jobSummary: RipJobSummary = {
          jobId,
          jobHeader: currentJob.jobHeader,
          totalTracks: currentJob.totalTracks,
          cachedCount,
          rippedCount: rippedCountInner,
          failedCount: failedTracks.length,
          failedTracks,
          skippedUncachedTracks: [],
          totalElapsedSec,
          cappedCount,
          maxCollectionLimit,
          isCacheOnly,
          isGroup,
        }

        return jobSummary
      })

      currentJob.completed = true
      this.emit('job:completed', currentJob, summary)
      return summary
    } catch (err: unknown) {
      currentJob.completed = true
      this.emit('job:failed', currentJob, err)
      throw err
    } finally {
      this.jobs.delete(currentJob.id)
    }
  }
}

export const ripOrchestrator = new RipOrchestrator()
export const activeJobs = ripOrchestrator.activeJobs
