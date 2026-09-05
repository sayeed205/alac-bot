import { existsSync, unlinkSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { BotKeyboard, html, type Message } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { formatDumpCaption } from '@/modules/alac/indexer.ts'
import { fetchAlbumTracks, fetchArtistTracks } from '@/modules/alac/itunes.ts'
import {
  extractBatchItems,
  type ParsedTargetItem,
  parseAlacInput,
} from '@/modules/alac/parser.ts'
import { fetchPlaylistTracks } from '@/modules/alac/playlist.ts'
import { abortableSleep, type TrackRipResult } from '@/modules/alac/ripper.ts'
import { settingsService as defaultSettingsService } from '@/modules/settings/service.ts'
import {
  debug,
  debugSpan,
  error,
  info,
  infoSpan,
  warn,
} from '@/utils/logger.ts'
import { formatByteProgress, renderProgressBar } from '@/utils/progress.ts'
import { editMessageSafe, sendTextSafe } from '@/utils/telegram.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

interface ResolvedTrackItem {
  id: string
  title?: string
  artist?: string
  storefront?: string
}

export interface ActiveRipJob {
  id: string
  chatId: number
  userId: number
  jobHeader: string
  totalTracks: number
  statusMsgId: number
  controller: AbortController
  isCancelled: boolean
  cancelledBy?: string
  cachedCount: number
  rippedCount: number
  failedCount: number
  completed: boolean
}

export const activeJobs = new Map<string, ActiveRipJob>()

export function registerRipCommand(ctx: CommandContext): void {
  const { dp, tg, service, ripper, queue, auth } = ctx
  const settings = ctx.settings ?? defaultSettingsService
  const uploadRetryBaseMs = ctx.uploadRetryBaseMs ?? env.ALAC_RETRY_BASE_MS

  dp.onNewMessage(
    filters.command([
      'alac',
      'rip',
      'batch',
      'dl',
      'download',
      'rerip',
      'cache',
      'dump',
    ]),
    async (msg) => {
      using _cmdSpan = infoSpan('alac').enter()

      const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
      if (!isAuthed) {
        debug('Unauthorized user attempted alac command', {
          user_id: msg.sender.id,
        })
        return
      }

      const commandText = msg.text
        .trim()
        .split(/\s+/)[0]
        ?.toLowerCase()
        .replace(/^\//, '')
      const isRerip = commandText?.includes('rerip')
      const isCacheOnly = commandText === 'cache' || commandText === 'dump'

      const isAdmin = auth.isAdmin(msg.sender.id)
      if (isCacheOnly && !isAdmin) {
        debug('Non-admin requested cache/dump command', {
          user_id: msg.sender.id,
        })
        await msg.replyText(
          parseDynamicHtml(
            '🔒 <b>Access Restricted:</b> Caching directly to dump channel is restricted to the bot owner.',
          ),
        )
        return
      }

      const replyMsg = await msg.getReplyTo().catch(() => null)

      // Check for .txt document attachment on command or replied message
      let documentContent: string | null = null
      const docMedia =
        msg.media?.type === 'document'
          ? msg.media
          : replyMsg?.media?.type === 'document'
            ? replyMsg.media
            : null

      if (docMedia) {
        const fileName = (docMedia.fileName || '').toLowerCase()
        const mimeType = (docMedia.mimeType || '').toLowerCase()
        if (fileName.endsWith('.txt') || mimeType === 'text/plain') {
          const tmpFile = path.join(
            os.tmpdir(),
            `batch_${Date.now()}_${Math.random().toString(36).slice(2)}.txt`,
          )
          try {
            await tg.downloadToFile(tmpFile, docMedia)
            documentContent = await Bun.file(tmpFile).text()
          } catch (err) {
            debug('Failed to download batch text document', {
              error: String(err),
            })
          } finally {
            if (existsSync(tmpFile)) {
              try {
                unlinkSync(tmpFile)
              } catch {}
            }
          }
        }
      }

      let parsedItems: ParsedTargetItem[] = []
      let isForce = isRerip
      let singleStorefront: string | undefined

      if (documentContent) {
        parsedItems = extractBatchItems(documentContent)
        const textTokens = msg.text.trim().split(/\\s+/)
        if (textTokens.includes('-f') || textTokens.includes('--force')) {
          isForce = true
        }
      } else {
        const parsed = parseAlacInput(msg.text, replyMsg?.text)
        if (parsed) {
          parsedItems = parsed.items
          isForce = isForce || parsed.force
          singleStorefront = parsed.storefront
        }
      }

      if (parsedItems.length === 0) {
        debug('Invalid alac input received', { text: msg.text })
        if (isCacheOnly) {
          await msg.replyText(
            parseDynamicHtml(
              '💾 <b>Apple Music Lossless Cacher (Admin)</b><br/><br/>' +
                '<blockquote><b>Usage:</b><br/>' +
                '• <b>Track:</b> <code>/cache &lt;link | id&gt;</code><br/>' +
                '• <b>Album:</b> <code>/cache &lt;album_link&gt;</code><br/>' +
                '• <b>Playlist:</b> <code>/cache &lt;playlist_link&gt;</code><br/>' +
                '• <b>Artist:</b> <code>/cache &lt;artist_link&gt;</code><br/>' +
                '• <b>Batch:</b> Send multiple links or attach a <code>.txt</code> file<br/>' +
                '• <b>Alias:</b> <code>/dump</code><br/>' +
                '• <b>Options:</b> <code>-f</code> <i>(force re-rip even if cached)</i><br/>' +
                '<i>Rips and seeds lossless audio directly into dump channel and database without sending files to chat.</i></blockquote>',
            ),
          )
        } else {
          await msg.replyText(
            parseDynamicHtml(
              '🎵 <b>Apple Music Lossless Ripper</b><br/><br/>' +
                '<blockquote><b>Supported Inputs:</b><br/>' +
                '• <b>Track:</b> <code>/alac &lt;link | id&gt;</code><br/>' +
                '• <b>Album:</b> <code>/alac &lt;album_link&gt;</code><br/>' +
                '• <b>Playlist:</b> <code>/alac &lt;playlist_link&gt;</code><br/>' +
                '• <b>Artist:</b> <code>/alac &lt;artist_link&gt;</code><br/>' +
                '• <b>Batch:</b> Send multiple links or attach a <code>.txt</code> file<br/>' +
                '• <b>Aliases:</b> <code>/rip</code>, <code>/batch</code>, <code>/dl</code>, <code>/download</code><br/>' +
                '• <b>Cancel:</b> <code>/cancel</code> or tap the Cancel button on any active download<br/>' +
                '• <b>Options:</b> <code>-f</code> <i>(force re-rip)</i></blockquote>',
            ),
          )
        }
        return
      }

      if (isForce && !isAdmin) {
        debug('Non-admin requested force re-rip', { user_id: msg.sender.id })
        await msg.replyText(
          parseDynamicHtml(
            '🔒 <b>Access Restricted:</b> Force re-rip is restricted to the bot owner.',
          ),
        )
        return
      }

      if (!isAdmin) {
        if (!settings.canServeCache(isAdmin)) {
          debug('Ripping paused by admin', { user_id: msg.sender.id })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Ripping is temporarily paused for maintenance.</b><br/>' +
                'Please check back later.',
            ),
          )
          return
        }

        const hasAlbum = parsedItems.some((item) => item.type === 'album')
        if (hasAlbum && !settings.canRipAlbum(isAdmin)) {
          debug('Album ripping disabled by admin', { user_id: msg.sender.id })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Album ripping is currently disabled by admin.</b><br/>' +
                'Please request individual tracks instead.',
            ),
          )
          return
        }

        const hasPlaylist = parsedItems.some((item) => item.type === 'playlist')
        if (hasPlaylist && !settings.canRipPlaylist(isAdmin)) {
          debug('Playlist ripping disabled by admin', {
            user_id: msg.sender.id,
          })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Playlist ripping is currently disabled by admin.</b><br/>' +
                'Please request individual tracks instead.',
            ),
          )
          return
        }

        const hasArtist = parsedItems.some((item) => item.type === 'artist')
        if (hasArtist && !settings.canRipArtist(isAdmin)) {
          debug('Artist ripping disabled by admin', {
            user_id: msg.sender.id,
          })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Artist ripping is currently disabled by admin.</b><br/>' +
                'Please request individual tracks or albums instead.',
            ),
          )
          return
        }

        if (documentContent && !settings.canRipTxt(isAdmin)) {
          debug('.TXT batch ripping disabled by admin', {
            user_id: msg.sender.id,
          })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>.TXT file ripping is currently disabled by admin.</b><br/>' +
                'Please request individual links instead.',
            ),
          )
          return
        }

        if (
          !documentContent &&
          parsedItems.length > 1 &&
          !settings.canRipMultiLink(isAdmin)
        ) {
          debug('Multi-link ripping disabled by admin', {
            user_id: msg.sender.id,
          })
          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Multi-link ripping is currently disabled by admin.</b><br/>' +
                'Please request tracks or collections one at a time.',
            ),
          )
          return
        }
      }

      const isGroup = msg.chat.id !== msg.sender.id
      let deliveryChatId = msg.chat.id

      // In group chats, verify user has started the bot in DM so files can be sent privately
      if (isGroup && !isCacheOnly) {
        try {
          await tg.sendText(
            msg.sender.id,
            parseDynamicHtml(
              `📥 <b>Download Queued:</b><br/>Tracks requested in <b>${html.escape(msg.chat.displayName || 'the group')}</b> will be delivered here!`,
            ),
            { silent: true },
          )
          deliveryChatId = msg.sender.id
        } catch (_err) {
          debug('Cannot send to user DM, prompting to start bot in private', {
            user_id: msg.sender.id,
          })
          const me = await tg.getMe().catch(() => null)
          const botUsername = me?.username || 'alac_bot'
          const keyboard = BotKeyboard.inline([
            [
              BotKeyboard.url(
                '👉 Start Bot in DM',
                `https://t.me/${botUsername}?start=start`,
              ),
            ],
          ])

          await msg.replyText(
            parseDynamicHtml(
              '⚠️ <b>Direct Message Required</b><br/><br/>' +
                'To keep this group clean, all audio files are sent directly to your private DM.<br/>' +
                'Please click the button below to start the bot in DM, then send your request again!',
            ),
            { replyMarkup: keyboard },
          )
          return
        }
      }

      // Resolve all items (tracks, albums, playlists, artists) into track IDs
      const resolvingStatus = await msg.replyText(
        parseDynamicHtml('🔍 <b>Resolving tracks from Apple Music...</b>'),
      )

      const jobId = `${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
      const jobController = new AbortController()

      const currentJob: ActiveRipJob = {
        id: jobId,
        chatId: msg.chat.id,
        userId: msg.sender.id,
        jobHeader: '',
        totalTracks: 0,
        statusMsgId: resolvingStatus.id,
        controller: jobController,
        isCancelled: false,
        cachedCount: 0,
        rippedCount: 0,
        failedCount: 0,
        completed: false,
      }
      activeJobs.set(jobId, currentJob)

      const tracksToProcess: ResolvedTrackItem[] = []
      const seenTrackIds = new Set<string>()
      let jobHeader = ''
      const failedItems: string[] = []

      for (const item of parsedItems) {
        if (currentJob.isCancelled || jobController.signal.aborted) break

        try {
          if (item.type === 'album') {
            const albumData = await fetchAlbumTracks(
              item.id,
              item.storefront || singleStorefront,
            )
            if (!jobHeader) {
              jobHeader = `Album: <b>${html.escape(albumData.album.title)}</b> by <b>${html.escape(albumData.album.artist)}</b>`
            }
            for (const t of albumData.tracks) {
              if (!seenTrackIds.has(t.id)) {
                seenTrackIds.add(t.id)
                tracksToProcess.push({
                  id: t.id,
                  title: t.title,
                  artist: t.artist,
                  storefront: item.storefront || singleStorefront,
                })
              }
            }
          } else if (item.type === 'playlist') {
            const playlistData = await fetchPlaylistTracks(
              item.id,
              item.storefront || singleStorefront,
            )
            if (!jobHeader) {
              jobHeader = `Playlist: <b>${html.escape(playlistData.title)}</b>${playlistData.curatorName ? ` (${html.escape(playlistData.curatorName)})` : ''}`
            }
            for (const t of playlistData.tracks) {
              if (!seenTrackIds.has(t.id)) {
                seenTrackIds.add(t.id)
                tracksToProcess.push({
                  id: t.id,
                  title: t.title,
                  artist: t.artist,
                  storefront: item.storefront || singleStorefront,
                })
              }
            }
          } else if (item.type === 'artist') {
            const artistData = await fetchArtistTracks(
              item.id,
              item.storefront || singleStorefront,
            )
            if (!jobHeader) {
              jobHeader = `Artist: <b>${html.escape(artistData.artistName)}</b> (Discography)`
            }
            for (const t of artistData.tracks) {
              if (!seenTrackIds.has(t.id)) {
                seenTrackIds.add(t.id)
                tracksToProcess.push({
                  id: t.id,
                  title: t.title,
                  artist: t.artist,
                  storefront: item.storefront || singleStorefront,
                })
              }
            }
          } else {
            // Single track
            if (!seenTrackIds.has(item.id)) {
              seenTrackIds.add(item.id)
              tracksToProcess.push({
                id: item.id,
                storefront: item.storefront || singleStorefront,
              })
            }
          }
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err)
          failedItems.push(`${item.type} ${item.id}: ${errMsg}`)
          error('Failed to resolve target item', {
            type: item.type,
            id: item.id,
            error: errMsg,
          })
        }
      }

      if (currentJob.isCancelled || jobController.signal.aborted) {
        activeJobs.delete(jobId)
        return
      }

      if (tracksToProcess.length === 0) {
        activeJobs.delete(jobId)
        const errorDetail =
          failedItems.length > 0
            ? `<br/><code>${html.escape(failedItems.join('\n'))}</code>`
            : ''
        const resText = parseDynamicHtml(
          `⚠️ <b>Failed to resolve any tracks:</b>${errorDetail}`,
        )
        const resEdited = await editMessageSafe(tg, {
          chatId: msg.chat.id,
          message: resolvingStatus.id,
          text: resText,
          block: true,
          maxWaitSec: 10,
        })
        if (!resEdited) {
          await sendTextSafe(tg, {
            chatId: msg.chat.id,
            text: resText,
            params: { replyTo: msg.id },
            block: true,
            maxWaitSec: 10,
          })
        }
        return
      }

      // Apply collection limits for non-admin users
      const maxCollectionLimit = settings.getMaxCollectionTracks()
      let cappedCount = 0
      if (
        !isAdmin &&
        maxCollectionLimit > 0 &&
        tracksToProcess.length > maxCollectionLimit
      ) {
        cappedCount = tracksToProcess.length - maxCollectionLimit
        tracksToProcess.splice(maxCollectionLimit)
      }

      const isMultiTrack = tracksToProcess.length > 1
      if (!jobHeader) {
        if (isCacheOnly) {
          jobHeader = isMultiTrack
            ? `Batch Cache: <b>${tracksToProcess.length} tracks</b>`
            : `Track Cache: <code>${tracksToProcess[0]?.id}</code>`
        } else {
          jobHeader = isMultiTrack
            ? `Batch: <b>${tracksToProcess.length} tracks</b>`
            : `Track ID: <code>${tracksToProcess[0]?.id}</code>`
        }
      }

      currentJob.jobHeader = jobHeader
      currentJob.totalTracks = tracksToProcess.length

      info(isCacheOnly ? 'Cache job queued' : 'Rip job queued', {
        jobId,
        tracksCount: tracksToProcess.length,
        force: isForce,
        isGroup,
        isCacheOnly,
        deliveryChatId: isCacheOnly ? 'dump_only' : deliveryChatId,
      })

      const statusMsgId = resolvingStatus.id
      let cachedCount = 0
      let rippedCount = 0
      const failedTracks: { id: string; error: string }[] = []
      const skippedUncachedTracks: string[] = []
      const jobStartTime = Date.now()
      let lastStatusUpdate = 0
      let lastStatusText = ''
      let isEditing = false
      let isEditBlocked = false

      const cancelKeyboard = BotKeyboard.inline([
        [BotKeyboard.callback('❌ Cancel Download', `cancel:${jobId}`)],
      ])

      const updateProgress = async (currentStatus: string, _force = false) => {
        if (
          currentJob.isCancelled ||
          jobController.signal.aborted ||
          isEditBlocked
        )
          return

        const total = tracksToProcess.length
        const completed =
          cachedCount +
          rippedCount +
          failedTracks.length +
          skippedUncachedTracks.length
        const percent = Math.min(100, Math.round((completed / total) * 100))
        const bar = renderProgressBar(completed, total, 10)

        const statusLine = isCacheOnly
          ? `<b>Status:</b> ⚡ ${cachedCount} cached • 🎵 ${rippedCount} seeded`
          : `<b>Status:</b> ⚡ ${cachedCount} cached • 🎵 ${rippedCount} ripped`

        const formatted =
          `${isCacheOnly ? '💾' : '📋'} <b>${jobHeader}</b><br/>` +
          `<b>Progress:</b> <code>${bar} ${completed}/${total} (${percent}%)</code><br/>` +
          `${statusLine}` +
          (skippedUncachedTracks.length > 0
            ? ` • 🟡 ${skippedUncachedTracks.length} skipped`
            : '') +
          (failedTracks.length > 0
            ? ` • ⚠️ ${failedTracks.length} failed`
            : '') +
          `<br/><b>Current:</b> ${currentStatus}` +
          (isGroup && !isCacheOnly
            ? '<br/><i>Files delivered to your private DM 📩</i>'
            : '')

        const now = Date.now()
        if (isEditing) return
        const minInterval = lastStatusUpdate === 0 ? 0 : 10000
        if (now - lastStatusUpdate < minInterval) return
        if (formatted === lastStatusText) return

        isEditing = true
        lastStatusUpdate = now
        lastStatusText = formatted

        try {
          const edited = await editMessageSafe(tg, {
            chatId: msg.chat.id,
            message: statusMsgId,
            text: parseDynamicHtml(formatted),
            replyMarkup: cancelKeyboard,
            block: false,
          })
          if (!edited) {
            isEditBlocked = true
          }
        } finally {
          isEditing = false
        }
      }

      await updateProgress('Checking local cache...', true)

      const isLiveRippingAllowed = settings.canRipLive(isAdmin)
      const allTrackIds = tracksToProcess.map((t) => t.id)
      const cachedTracksMap = !isForce
        ? await service.findCachedTracks(allTrackIds)
        : new Map()

      // In cache-only mode for a single track: reject early if not cached
      if (
        !isLiveRippingAllowed &&
        tracksToProcess.length === 1 &&
        !cachedTracksMap.has(tracksToProcess[0]?.id)
      ) {
        activeJobs.delete(jobId)
        const maintText = parseDynamicHtml(
          '⚠️ <b>Live ripping is currently disabled for maintenance.</b><br/>' +
            'This track is not yet in the local cache. Only cached tracks can be played right now.',
        )
        const maintEdited = await editMessageSafe(tg, {
          chatId: msg.chat.id,
          message: resolvingStatus.id,
          text: maintText,
          block: true,
          maxWaitSec: 10,
        })
        if (!maintEdited) {
          await sendTextSafe(tg, {
            chatId: msg.chat.id,
            text: maintText,
            params: { replyTo: msg.id },
            block: true,
            maxWaitSec: 10,
          })
        }
        return
      }

      for (let index = 0; index < tracksToProcess.length; index++) {
        if (currentJob.isCancelled || jobController.signal.aborted) {
          break
        }

        const item = tracksToProcess[index]
        if (!item) continue
        const trackId = item.id

        using _trackSpan = debugSpan('track_job', { track_id: trackId }).enter()
        const startTime = Date.now()

        if (!isForce) {
          const cached = cachedTracksMap.get(trackId)
          if (cached) {
            if (isCacheOnly) {
              cachedCount++
              currentJob.cachedCount = cachedCount
              await updateProgress(`⚡ Already cached: ${trackId}`)
              continue
            }

            try {
              await tg.sendCopy({
                toChatId: deliveryChatId,
                fromChatId: env.DUMP_CHANNEL_ID,
                message: cached.messageId,
                ...(deliveryChatId === msg.chat.id ? { replyTo: msg.id } : {}),
                silent: isMultiTrack,
              })

              const durationMs = Date.now() - startTime
              info('Cache hit: delivered', {
                track_id: trackId,
                time: `${durationMs}ms`,
              })

              await service.logRequest({
                telegramId: msg.sender.id,
                chatId: msg.chat.id,
                appleTrackId: trackId,
                isCacheHit: true,
                durationMs,
                status: 'completed',
              })

              cachedCount++
              currentJob.cachedCount = cachedCount
              await updateProgress(`⚡ Cache hit delivered: ${trackId}`)
              continue
            } catch (copyErr) {
              debug('Cache forward failed, falling back to ripper', {
                track_id: trackId,
                error: String(copyErr),
              })
            }
          }
        }

        // Check if live ripping is disallowed (cache-only mode)
        if (!isLiveRippingAllowed) {
          skippedUncachedTracks.push(trackId)
          await updateProgress(
            `🟡 Skipped (uncached in cache-only mode): ${trackId}`,
          )
          continue
        }

        // Rip via queue
        try {
          await queue.enqueue(
            async (taskSignal) => {
              if (currentJob.isCancelled || taskSignal.aborted) {
                throw new Error('Download was cancelled')
              }

              await updateProgress(
                `Connecting mirror for track #${index + 1}...`,
                true,
              )

              let ripResult: TrackRipResult | null = null

              try {
                ripResult = await ripper.rip(
                  trackId,
                  async (status) => {
                    await updateProgress(status)
                  },
                  item.storefront,
                  taskSignal,
                )

                if (currentJob.isCancelled || taskSignal.aborted) {
                  throw new Error('Download was cancelled')
                }

                await updateProgress('📤 <b>Uploading to Telegram...</b>', true)
                debug('Uploading track to dump channel', {
                  track_id: trackId,
                  file: ripResult.filePath,
                })

                const caption = formatDumpCaption({
                  appleTrackId: trackId,
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

                let dumpMsg: Message | null = null
                let uploadAttempt = 0
                const maxUploadRetries = env.ALAC_MAX_RETRIES

                while (true) {
                  if (currentJob.isCancelled || taskSignal.aborted) {
                    throw new Error('Download was cancelled')
                  }

                  try {
                    dumpMsg = await tg.sendMedia(
                      env.DUMP_CHANNEL_ID,
                      {
                        type: 'audio',
                        file: Bun.file(ripResult.filePath),
                        title: ripResult.title,
                        performer: ripResult.artist,
                        duration: ripResult.duration,
                        caption,
                      },
                      {
                        progressCallback: (uploaded, total) => {
                          if (total > 0) {
                            const progressText = formatByteProgress(
                              uploaded,
                              total,
                              10,
                            )
                            updateProgress(
                              `📤 <b>Uploading:</b> <code>${progressText}</code>`,
                            )
                          }
                        },
                      },
                    )
                    break
                  } catch (uploadErr: unknown) {
                    if (
                      currentJob.isCancelled ||
                      taskSignal.aborted ||
                      (uploadErr instanceof Error &&
                        uploadErr.message === 'Download was cancelled')
                    ) {
                      throw uploadErr
                    }

                    if (uploadAttempt >= maxUploadRetries) {
                      throw uploadErr
                    }

                    uploadAttempt++
                    const rawDelay =
                      uploadRetryBaseMs * 2 ** (uploadAttempt - 1)
                    const jitterFactor = 0.8 + Math.random() * 0.4
                    const delayMs = Math.round(rawDelay * jitterFactor)
                    const errMsg =
                      uploadErr instanceof Error
                        ? uploadErr.message
                        : String(uploadErr)
                    const waitSec = (delayMs / 1000).toFixed(1)

                    await updateProgress(
                      `⚠️ <b>Upload failed, retrying (${uploadAttempt}/${maxUploadRetries}) in ${waitSec}s:</b> <code>${html.escape(errMsg)}</code>`,
                    )
                    warn('Track upload to dump failed, retrying', {
                      track_id: trackId,
                      attempt: uploadAttempt,
                      max_retries: maxUploadRetries,
                      delay_ms: delayMs,
                      error: errMsg,
                    })

                    await abortableSleep(delayMs, taskSignal)
                  }
                }

                if (currentJob.isCancelled || taskSignal.aborted) {
                  throw new Error('Download was cancelled')
                }

                let fileId = ''
                let fileUniqueId = ''
                if (dumpMsg.media && dumpMsg.media.type === 'audio') {
                  fileId = dumpMsg.media.fileId
                  fileUniqueId = dumpMsg.media.uniqueFileId
                }

                await service.saveTrack({
                  appleTrackId: trackId,
                  messageId: dumpMsg.id,
                  fileId,
                  fileUniqueId,
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
                  await tg.sendCopy({
                    toChatId: deliveryChatId,
                    fromChatId: env.DUMP_CHANNEL_ID,
                    message: dumpMsg.id,
                    ...(deliveryChatId === msg.chat.id
                      ? { replyTo: msg.id }
                      : {}),
                    silent: isMultiTrack,
                  })
                }

                rippedCount++
                currentJob.rippedCount = rippedCount
                const totalDurationMs = Date.now() - startTime
                info(isCacheOnly ? 'Track cached to dump' : 'Track completed', {
                  track: `${ripResult.artist} - ${ripResult.title}`,
                  time: `${(totalDurationMs / 1000).toFixed(1)}s`,
                })

                await service.logRequest({
                  telegramId: msg.sender.id,
                  chatId: msg.chat.id,
                  appleTrackId: trackId,
                  isCacheHit: false,
                  durationMs: totalDurationMs,
                  status: 'completed',
                })
              } finally {
                if (ripResult && existsSync(ripResult.filePath)) {
                  try {
                    unlinkSync(ripResult.filePath)
                  } catch {}
                }
              }
            },
            {
              signal: jobController.signal,
              onPositionChange: (pos) => {
                debug('Queue position changed', { track_id: trackId, pos })
                updateProgress(
                  `⏳ <b>In Queue:</b> Position <code>#${pos}</code>`,
                  true,
                )
              },
              onStart: () => {
                debug('Track rip started by queue worker', {
                  track_id: trackId,
                })
              },
            },
          )
        } catch (err: unknown) {
          if (currentJob.isCancelled || jobController.signal.aborted) {
            break
          }

          const errMsg = err instanceof Error ? err.message : String(err)
          failedTracks.push({ id: trackId, error: errMsg })
          currentJob.failedCount = failedTracks.length
          error('Rip job failed', {
            track_id: trackId,
            duration_ms: Date.now() - startTime,
            error: errMsg,
            stack: err instanceof Error ? err.stack : undefined,
          })

          await service.logRequest({
            telegramId: msg.sender.id,
            chatId: msg.chat.id,
            appleTrackId: trackId,
            isCacheHit: false,
            durationMs: Date.now() - startTime,
            status: 'failed',
            errorReason: errMsg,
          })

          await updateProgress(`⚠️ Track failed: ${trackId}`)

          // Circuit breaker: If the mirror server is down or unreachable, abort remaining batch
          const isMirrorDown =
            errMsg.includes('Mirror /status check timed out') ||
            errMsg.includes('Mirror health check failed') ||
            errMsg.includes('Lossless wrapper is currently offline') ||
            errMsg.includes('Mirror manifest lookup timed out') ||
            errMsg.includes('Mirror service is currently offline') ||
            errMsg.includes('Failed to connect to mirror stream')

          if (isMirrorDown) {
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

      currentJob.completed = true
      activeJobs.delete(jobId)

      if (currentJob.isCancelled) {
        return
      }

      // Final completion card (no cancel button)
      const totalElapsedSec = ((Date.now() - jobStartTime) / 1000).toFixed(1)
      const totalTracks = tracksToProcess.length

      let summaryHtml: string
      if (isCacheOnly) {
        summaryHtml =
          `✅ <b>Caching Complete!</b><br/><br/>` +
          `<blockquote>• <b>Target:</b> ${jobHeader}<br/>` +
          `• <b>Total Tracks:</b> <code>${totalTracks}</code><br/>` +
          `• <b>Seeded to Dump:</b> 🎵 <code>${rippedCount}</code> new • ⚡ <code>${cachedCount}</code> already cached<br/>` +
          (failedTracks.length > 0
            ? `• <b>Failed:</b> ⚠️ <code>${failedTracks.length}</code><br/>`
            : '') +
          `• <b>Time Elapsed:</b> <code>${totalElapsedSec}s</code><br/>` +
          `• <b>Destination:</b> Dump Channel & Database</blockquote>`
      } else if (
        cachedCount === 0 &&
        rippedCount === 0 &&
        skippedUncachedTracks.length > 0
      ) {
        summaryHtml =
          '⚠️ <b>No Cached Tracks Available</b><br/><br/>' +
          `<blockquote>• <b>Target:</b> ${jobHeader}<br/>` +
          `• <b>Total Requested:</b> <code>${totalTracks}</code><br/>` +
          `• <b>Skipped (Uncached):</b> 🟡 <code>${skippedUncachedTracks.length}</code><br/>` +
          '• <b>Note:</b> Live ripping is currently disabled for maintenance.</blockquote>'
      } else {
        summaryHtml =
          `✅ <b>Download Complete!</b><br/><br/>` +
          `<blockquote>• <b>Target:</b> ${jobHeader}<br/>` +
          `• <b>Total Tracks:</b> <code>${totalTracks}</code><br/>` +
          `• <b>Delivered:</b> ⚡ <code>${cachedCount}</code> cached • 🎵 <code>${rippedCount}</code> ripped<br/>` +
          (skippedUncachedTracks.length > 0
            ? `• <b>Skipped (Uncached):</b> 🟡 <code>${skippedUncachedTracks.length}</code><br/>`
            : '') +
          (failedTracks.length > 0
            ? `• <b>Failed:</b> ⚠️ <code>${failedTracks.length}</code><br/>`
            : '') +
          `• <b>Time Elapsed:</b> <code>${totalElapsedSec}s</code></blockquote>`
      }

      if (cappedCount > 0) {
        summaryHtml += `<br/>ℹ️ <i>Queue was capped to ${maxCollectionLimit} tracks (settings limit).</i>`
      }

      if (isGroup && !isCacheOnly) {
        summaryHtml +=
          '<br/>📩 <i>All songs have been delivered to your private DM!</i>'
      }

      if (failedTracks.length > 0) {
        summaryHtml += '<br/><br/><b>Issues / Failures:</b><br/>'
        for (const f of failedTracks.slice(0, 5)) {
          summaryHtml += `• <code>${f.id}</code>: ${html.escape(f.error)}<br/>`
        }
        if (failedTracks.length > 5) {
          summaryHtml += `<i>...and ${failedTracks.length - 5} more</i>`
        }
      }

      let summaryEdited = false
      if (!isEditBlocked) {
        summaryEdited = await editMessageSafe(tg, {
          chatId: msg.chat.id,
          message: statusMsgId,
          text: parseDynamicHtml(summaryHtml),
          block: true,
          maxWaitSec: 10,
        })
      }

      if (!summaryEdited) {
        await sendTextSafe(tg, {
          chatId: msg.chat.id,
          text: parseDynamicHtml(summaryHtml),
          params: { replyTo: msg.id },
          block: true,
          maxWaitSec: 10,
        })
      }
    },
  )

  // /cancel command handler
  dp.onNewMessage(filters.command('cancel'), async (msg) => {
    const callerId = msg.sender.id
    const isAdmin = auth.isAdmin(callerId)

    let targetJob: ActiveRipJob | undefined
    for (const job of activeJobs.values()) {
      if (job.chatId === msg.chat.id && (job.userId === callerId || isAdmin)) {
        targetJob = job
        break
      }
    }

    if (!targetJob || targetJob.completed || targetJob.isCancelled) {
      await msg.replyText(
        parseDynamicHtml('ℹ️ <b>No active download to cancel in this chat.</b>'),
      )
      return
    }

    const cancellerName = msg.sender.displayName || (isAdmin ? 'Admin' : 'User')
    targetJob.isCancelled = true
    targetJob.cancelledBy = cancellerName
    targetJob.controller.abort()
    activeJobs.delete(targetJob.id)

    await msg.replyText(
      parseDynamicHtml('🛑 <b>Download has been cancelled.</b>'),
    )

    const processed = targetJob.cachedCount + targetJob.rippedCount
    const cancelledHtml =
      `🛑 <b>Download Cancelled</b><br/><br/>` +
      `<blockquote>• <b>Target:</b> ${targetJob.jobHeader || 'Download'}<br/>` +
      `• <b>Cancelled by:</b> <b>${html.escape(cancellerName)}</b><br/>` +
      `• <b>Progress when cancelled:</b> <code>${processed}/${targetJob.totalTracks} tracks processed</code></blockquote>`

    const cancelEdited = await editMessageSafe(tg, {
      chatId: targetJob.chatId,
      message: targetJob.statusMsgId,
      text: parseDynamicHtml(cancelledHtml),
      block: true,
      maxWaitSec: 5,
    })
    if (!cancelEdited) {
      await sendTextSafe(tg, {
        chatId: targetJob.chatId,
        text: parseDynamicHtml(cancelledHtml),
        block: true,
        maxWaitSec: 5,
      })
    }
  })

  // Interactive Cancel button callback query handler
  dp.onCallbackQuery(filters.startsWith('cancel:'), async (query) => {
    const jobId = query.dataStr?.replace('cancel:', '').trim()
    if (!jobId) return

    const job = activeJobs.get(jobId)
    if (!job || job.completed) {
      await query.answer({
        text: '⚠️ This download has already completed or expired.',
      })
      return
    }

    const callerId = query.user.id
    const isAdmin = auth.isAdmin(callerId)
    const isOwner = callerId === job.userId

    if (!isAdmin && !isOwner) {
      await query.answer({
        text: '⛔ Only the person who requested this download or an admin can cancel it.',
        alert: true,
      })
      return
    }

    const cancellerName = query.user.displayName || (isAdmin ? 'Admin' : 'User')
    job.isCancelled = true
    job.cancelledBy = cancellerName
    job.controller.abort()
    activeJobs.delete(jobId)

    await query.answer({ text: '🛑 Download cancelled.' })

    const processed = job.cachedCount + job.rippedCount
    const cancelledHtml =
      `🛑 <b>Download Cancelled</b><br/><br/>` +
      `<blockquote>• <b>Target:</b> ${job.jobHeader || 'Download'}<br/>` +
      `• <b>Cancelled by:</b> <b>${html.escape(cancellerName)}</b><br/>` +
      `• <b>Progress when cancelled:</b> <code>${processed}/${job.totalTracks} tracks processed</code></blockquote>`

    const cancelEdited = await editMessageSafe(tg, {
      chatId: job.chatId,
      message: job.statusMsgId,
      text: parseDynamicHtml(cancelledHtml),
      block: true,
      maxWaitSec: 5,
    })
    if (!cancelEdited) {
      await sendTextSafe(tg, {
        chatId: job.chatId,
        text: parseDynamicHtml(cancelledHtml),
        block: true,
        maxWaitSec: 5,
      })
    }
  })
}
