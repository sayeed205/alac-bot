import { existsSync, rmSync, unlinkSync } from 'node:fs'
import { mkdir } from 'node:fs/promises'
import os from 'node:os'
import path, { join } from 'node:path'

import { BotKeyboard, html, type Message } from '@mtcute/bun'
import { filters, type MessageContext } from '@mtcute/dispatcher'

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
import { BoundedChannel } from '@/utils/channel.ts'
import type { FormattedString } from '@/utils/html.ts'
import {
  debug,
  debugSpan,
  error,
  info,
  infoSpan,
  warn,
} from '@/utils/logger.ts'
import { formatMbProgress, renderProgressBar } from '@/utils/progress.ts'
import { editMessageSafe, sendTextSafe } from '@/utils/telegram.ts'

import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

interface ResolvedTrackItem {
  id: string
  title?: string
  artist?: string
  storefront?: string
}

interface PipelineItem {
  index: number
  item: ResolvedTrackItem
  status:
    | { type: 'ready'; ripResult: TrackRipResult; startTime: number }
    | {
        type: 'failed'
        error: string
        isMirrorDown?: boolean
        startTime: number
      }
}

export interface ActiveRipJob {
  id: string
  chatId: number
  userId: number
  userName?: string
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
  queuePosition?: number
  startTime: number
  activeActionText?: string
}

export const activeJobs = new Map<string, ActiveRipJob>()

export function registerRipCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx
  const settings = ctx.settings ?? defaultSettingsService

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

      await executeRipPipeline(ctx, {
        chatId: msg.chat.id,
        userId: msg.sender.id,
        displayName: msg.chat.displayName,
        replyToMessageId: msg.id,
        parsedItems,
        isCacheOnly,
        isForce,
        singleStorefront,
        msg,
      })
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

export interface ExecuteRipPipelineOptions {
  chatId: number
  userId: number
  displayName?: string
  replyToMessageId?: number
  parsedItems: ParsedTargetItem[]
  isCacheOnly: boolean
  isForce?: boolean
  singleStorefront?: string
  msg?: MessageContext
  statusMessageToReuse?: Message
}

export async function executeRipPipeline(
  ctx: CommandContext,
  options: ExecuteRipPipelineOptions,
): Promise<void> {
  const { tg, service, ripper, queue, auth } = ctx
  const settings = ctx.settings ?? defaultSettingsService
  const uploadRetryBaseMs = ctx.uploadRetryBaseMs ?? env.ALAC_RETRY_BASE_MS
  const {
    chatId,
    userId,
    displayName,
    replyToMessageId,
    parsedItems,
    isCacheOnly,
    isForce = false,
    singleStorefront,
    msg,
  } = options

  const isAdmin = auth.isAdmin(userId)
  const isGroup = chatId !== userId
  let deliveryChatId = chatId

  // In group chats, verify user has started the bot in DM so files can be sent privately
  if (isGroup && !isCacheOnly) {
    try {
      await tg.sendText(
        userId,
        parseDynamicHtml(
          `📥 <b>Download Queued:</b><br/>Tracks requested in <b>${html.escape(displayName || 'the group' || 'the group')}</b> will be delivered here!`,
        ),
        { silent: true },
      )
      deliveryChatId = userId
    } catch (_err) {
      debug('Cannot send to user DM, prompting to start bot in private', {
        user_id: userId,
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

      if (msg) {
        await msg.replyText(
          parseDynamicHtml(
            '⚠️ <b>Direct Message Required</b><br/><br/>' +
              'To keep this group clean, all audio files are sent directly to your private DM.<br/>' +
              'Please click the button below to start the bot in DM, then send your request again!',
          ),
          { replyMarkup: keyboard },
        )
      } else {
        await tg.sendText(
          chatId,
          parseDynamicHtml(
            '⚠️ <b>Direct Message Required</b><br/><br/>' +
              'To keep this group clean, all audio files are sent directly to your private DM.<br/>' +
              'Please click the button below to start the bot in DM, then send your request again!',
          ),
          { replyMarkup: keyboard, replyTo: replyToMessageId },
        )
      }
      return
    }
  }

  // Resolve all items (tracks, albums, playlists, artists) into track IDs
  let resolvingStatus: Message
  if (options.statusMessageToReuse) {
    resolvingStatus = options.statusMessageToReuse
    await editMessageSafe(tg, {
      chatId,
      message: resolvingStatus.id,
      text: parseDynamicHtml('🔍 <b>Resolving tracks from Apple Music...</b>'),
      block: true,
      maxWaitSec: 10,
    })
  } else if (msg) {
    resolvingStatus = await msg.replyText(
      parseDynamicHtml('🔍 <b>Resolving tracks from Apple Music...</b>'),
    )
  } else {
    resolvingStatus = await tg.sendText(
      chatId,
      parseDynamicHtml('🔍 <b>Resolving tracks from Apple Music...</b>'),
      replyToMessageId ? { replyTo: replyToMessageId } : undefined,
    )
  }

  const jobId = `${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
  const jobController = new AbortController()

  const requesterName =
    displayName ||
    msg?.sender?.displayName ||
    (msg?.sender?.username ? `@${msg.sender.username}` : `User ${userId}`)

  const currentJob: ActiveRipJob = {
    id: jobId,
    chatId: chatId,
    userId: userId,
    userName: requesterName,
    jobHeader: '',
    totalTracks: 0,
    statusMsgId: resolvingStatus.id,
    controller: jobController,
    isCancelled: false,
    cachedCount: 0,
    rippedCount: 0,
    failedCount: 0,
    completed: false,
    startTime: Date.now(),
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
      chatId: chatId,
      message: resolvingStatus.id,
      text: resText,
      block: true,
      maxWaitSec: 10,
    })
    if (!resEdited) {
      await sendTextSafe(tg, {
        chatId: chatId,
        text: resText,
        params: { replyTo: replyToMessageId },
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

  let activeDownloadText = ''
  let activeUploadText = ''

  const updateProgress = async (activityOverride?: string, force = false) => {
    if (currentJob.isCancelled || jobController.signal.aborted || isEditBlocked)
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

    let activityText = ''
    if (activeDownloadText && activeUploadText) {
      activityText = `<br/>${activeDownloadText}<br/>${activeUploadText}`
    } else if (activeDownloadText) {
      activityText = `<br/>${activeDownloadText}`
    } else if (activeUploadText) {
      activityText = `<br/>${activeUploadText}`
    } else if (activityOverride) {
      activityText = `<br/><b>Current:</b> ${activityOverride}`
    }

    const formatted =
      `${isCacheOnly ? '💾' : '📋'} <b>${jobHeader}</b><br/>` +
      `<b>Progress:</b> <code>${bar} ${completed}/${total} (${percent}%)</code><br/>` +
      `${statusLine}` +
      (skippedUncachedTracks.length > 0
        ? ` • 🟡 ${skippedUncachedTracks.length} skipped`
        : '') +
      (failedTracks.length > 0 ? ` • ⚠️ ${failedTracks.length} failed` : '') +
      activityText +
      (isGroup && !isCacheOnly
        ? '<br/><i>Files delivered to your private DM 📩</i>'
        : '')

    const now = Date.now()
    if (isEditing) return
    const minInterval = lastStatusUpdate === 0 ? 0 : 10000
    if (!force && now - lastStatusUpdate < minInterval) return
    if (formatted === lastStatusText) return

    isEditing = true
    lastStatusUpdate = now
    lastStatusText = formatted

    try {
      const edited = await editMessageSafe(tg, {
        chatId: chatId,
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

  const sendFinalSummary = async () => {
    if (currentJob.isCancelled) {
      return
    }

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
        chatId: chatId,
        message: statusMsgId,
        text: parseDynamicHtml(summaryHtml),
        block: true,
        maxWaitSec: 10,
      })
    }

    if (!summaryEdited) {
      await sendTextSafe(tg, {
        chatId: chatId,
        text: parseDynamicHtml(summaryHtml),
        params: { replyTo: replyToMessageId },
        block: true,
        maxWaitSec: 10,
      })
    }
  }

  const allTrackIds = tracksToProcess.map((t) => t.id)
  let itemsToRip: ResolvedTrackItem[] = []

  if (isForce) {
    // Force re-rip: delete existing cache and dump messages first prior to queueing
    const existingTracksMap = await service.findCachedTracks(allTrackIds)
    const oldMsgIds = [...existingTracksMap.values()]
      .map((t) => t.messageId)
      .filter((id): id is number => typeof id === 'number' && id > 0)

    if (oldMsgIds.length > 0) {
      debug('Deleting old dump messages on force re-rip prior to queue', {
        count: oldMsgIds.length,
        old_message_ids: oldMsgIds,
      })
      await tg
        .deleteMessagesById(env.DUMP_CHANNEL_ID, oldMsgIds)
        .catch((err) => {
          warn('Failed to delete old dump messages on force re-rip', {
            error: String(err),
          })
        })
    }

    for (const trackId of existingTracksMap.keys()) {
      await service.deleteTrack(trackId).catch((err) => {
        warn('Failed to delete track from DB on force re-rip', {
          track_id: trackId,
          error: String(err),
        })
      })
    }

    itemsToRip = [...tracksToProcess]
  } else {
    // Normal rip/cache: check cache immediately before queue
    await updateProgress('Checking local cache...', true)
    const existingTracksMap = await service.findCachedTracks(allTrackIds)
    const cachedItems: ResolvedTrackItem[] = []
    const uncachedItems: ResolvedTrackItem[] = []

    for (const item of tracksToProcess) {
      if (existingTracksMap.has(item.id)) {
        cachedItems.push(item)
      } else {
        uncachedItems.push(item)
      }
    }

    // Deliver / record cached tracks immediately without waiting in queue
    if (cachedItems.length > 0) {
      for (const item of cachedItems) {
        if (currentJob.isCancelled || jobController.signal.aborted) {
          break
        }

        const cached = existingTracksMap.get(item.id)
        if (!cached) continue

        if (isCacheOnly) {
          cachedCount++
          currentJob.cachedCount = cachedCount
          info('Track already cached in dump channel', { track_id: item.id })
          await updateProgress('Recognized cached tracks...', true)
        } else {
          try {
            await tg.sendCopy({
              toChatId: deliveryChatId,
              fromChatId: env.DUMP_CHANNEL_ID,
              message: cached.messageId,
              caption: { text: '' },
              ...(deliveryChatId === chatId
                ? { replyTo: replyToMessageId }
                : {}),
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
            await updateProgress('Delivered cached track...', true)
          } catch (copyErr) {
            debug('Cache forward failed, treating as uncached', {
              track_id: item.id,
              error: String(copyErr),
            })
            // If the message is missing from dump channel, fall back to ripping fresh
            uncachedItems.push(item)
          }
        }
      }
    }

    // If all requested tracks were delivered/cached, finish immediately!
    if (uncachedItems.length === 0 || currentJob.isCancelled) {
      await sendFinalSummary()
      currentJob.completed = true
      activeJobs.delete(jobId)
      return
    }

    // Uncached tracks remain. Check live ripping permissions:
    const isLiveRippingAllowed = settings.canRipLive(isAdmin)
    if (!isLiveRippingAllowed) {
      if (tracksToProcess.length === 1) {
        const maintText = parseDynamicHtml(
          '⚠️ <b>Live ripping is currently disabled for maintenance.</b><br/>' +
            'This track is not yet in the local cache. Only cached tracks can be played right now.',
        )
        const maintEdited = await editMessageSafe(tg, {
          chatId: chatId,
          message: resolvingStatus.id,
          text: maintText,
          block: true,
          maxWaitSec: 10,
        })
        if (!maintEdited) {
          await sendTextSafe(tg, {
            chatId: chatId,
            text: maintText,
            params: { replyTo: replyToMessageId },
            block: true,
            maxWaitSec: 10,
          })
        }
        currentJob.completed = true
        activeJobs.delete(jobId)
        return
      }

      // Multi-track: skip uncached tracks and send summary of cached ones
      skippedUncachedTracks.push(...uncachedItems.map((t) => t.id))
      await sendFinalSummary()
      currentJob.completed = true
      activeJobs.delete(jobId)
      return
    }

    itemsToRip = uncachedItems
  }

  const queueStatusPrefix = cachedCount > 0 ? `⚡ ${cachedCount} cached • ` : ''
  await updateProgress(
    `${queueStatusPrefix}⏳ ${itemsToRip.length} track${itemsToRip.length === 1 ? '' : 's'} waiting in queue...`,
    true,
  )

  try {
    await queue.enqueue(
      async (taskSignal) => {
        if (
          currentJob.isCancelled ||
          jobController.signal.aborted ||
          taskSignal.aborted
        ) {
          return
        }

        const jobTempDir = join(os.tmpdir(), `alac_job_${jobId}`)
        await mkdir(jobTempDir, { recursive: true })

        const channel = new BoundedChannel<PipelineItem>(
          2,
          jobController.signal,
        )

        const downloadTask = async () => {
          try {
            for (let index = 0; index < itemsToRip.length; index++) {
              if (
                currentJob.isCancelled ||
                jobController.signal.aborted ||
                taskSignal.aborted
              ) {
                break
              }

              const item = itemsToRip[index]
              if (!item) continue
              const trackId = item.id

              const itemTitle = item.title
                ? `${item.artist || 'Unknown'} - ${item.title}`
                : `Track #${index + 1}`
              const startTime = Date.now()

              try {
                if (
                  currentJob.isCancelled ||
                  jobController.signal.aborted ||
                  taskSignal.aborted
                ) {
                  throw new Error('Download was cancelled')
                }

                activeDownloadText = `📥 <b>Downloading:</b> ${html.escape(itemTitle)}`
                currentJob.activeActionText = activeDownloadText
                await updateProgress()

                const ripResult = await ripper.rip(
                  trackId,
                  (status, downloadedBytes, totalBytes) => {
                    if (
                      downloadedBytes !== undefined &&
                      totalBytes !== undefined &&
                      totalBytes > 0
                    ) {
                      const mbProgress = formatMbProgress(
                        downloadedBytes,
                        totalBytes,
                      )
                      activeDownloadText = `📥 <b>Downloading:</b> ${html.escape(itemTitle)} <code>[${mbProgress}]</code>`
                    } else if (status.includes('Tagging')) {
                      activeDownloadText = `🏷️ <b>Tagging:</b> ${html.escape(itemTitle)}`
                    } else {
                      activeDownloadText = `📥 <b>Downloading:</b> ${html.escape(itemTitle)}`
                    }
                    currentJob.activeActionText = activeDownloadText
                    updateProgress().catch(() => {})
                  },
                  item.storefront,
                  taskSignal,
                )

                activeDownloadText = ''
                currentJob.activeActionText = ''
                await channel.push({
                  index,
                  item,
                  status: { type: 'ready', ripResult, startTime },
                })
              } catch (err: unknown) {
                activeDownloadText = ''
                currentJob.activeActionText = ''
                if (
                  currentJob.isCancelled ||
                  jobController.signal.aborted ||
                  taskSignal.aborted
                ) {
                  break
                }

                const errMsg = err instanceof Error ? err.message : String(err)
                const isMirrorDown =
                  errMsg.includes('Mirror /status check timed out') ||
                  errMsg.includes('Mirror health check failed') ||
                  errMsg.includes('Lossless wrapper is currently offline') ||
                  errMsg.includes('Mirror manifest lookup timed out') ||
                  errMsg.includes('Mirror service is currently offline') ||
                  errMsg.includes('Failed to connect to mirror stream')

                await channel.push({
                  index,
                  item,
                  status: {
                    type: 'failed',
                    error: errMsg,
                    isMirrorDown,
                    startTime,
                  },
                })

                if (isMirrorDown) {
                  break
                }
              }
            }
          } finally {
            activeDownloadText = ''
            channel.close()
          }
        }

        const uploadTask = async () => {
          try {
            while (true) {
              if (currentJob.isCancelled || jobController.signal.aborted) {
                break
              }

              const pipelineItem = await channel.pull()
              if (!pipelineItem) {
                break
              }

              const { index, item, status } = pipelineItem
              const trackId = item.id
              const itemTitle = item.title
                ? `${item.artist || 'Unknown'} - ${item.title}`
                : `Track #${index + 1}`

              using _trackSpan = debugSpan('track_job', {
                track_id: trackId,
              }).enter()

              if (status.type === 'failed') {
                failedTracks.push({ id: trackId, error: status.error })
                currentJob.failedCount = failedTracks.length
                error('Rip job failed', {
                  track_id: trackId,
                  duration_ms: Date.now() - status.startTime,
                  error: status.error,
                })

                await service.logRequest({
                  telegramId: userId,
                  chatId: chatId,
                  appleTrackId: trackId,
                  isCacheHit: false,
                  durationMs: Date.now() - status.startTime,
                  status: 'failed',
                  errorReason: status.error,
                })

                await updateProgress()

                if (status.isMirrorDown) {
                  failedTracks.push({
                    id: 'Remaining tracks',
                    error:
                      'Mirror service offline / unreachable (stopped remaining batch)',
                  })
                  error(
                    'Lossless mirror appears to be down, aborting remaining batch to prevent repeated timeouts',
                    { track_id: trackId, error: status.error },
                  )
                  jobController.abort()
                  break
                }
                continue
              }

              if (status.type === 'ready') {
                const { ripResult, startTime } = status
                try {
                  activeUploadText = `📤 <b>Uploading:</b> ${html.escape(itemTitle)}`
                  await updateProgress()

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

                  let currentCaption: FormattedString | string = caption
                  let dumpMsg: Message | null = null
                  activeUploadText = `📤 <b>Uploading:</b> ${html.escape(itemTitle)}`
                  currentJob.activeActionText = activeUploadText
                  let uploadAttempt = 0
                  const maxUploadRetries = env.ALAC_MAX_RETRIES

                  while (true) {
                    if (
                      currentJob.isCancelled ||
                      jobController.signal.aborted
                    ) {
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
                          caption: currentCaption,
                        },
                        {
                          silent: true,
                          progressCallback: (uploaded, total) => {
                            if (total > 0) {
                              const mbProgress = formatMbProgress(
                                uploaded,
                                total,
                              )
                              activeUploadText = `📤 <b>Uploading:</b> ${html.escape(itemTitle)} <code>[${mbProgress}]</code>`
                              currentJob.activeActionText = activeUploadText
                              updateProgress().catch(() => {})
                            }
                          },
                        },
                      )
                      break
                    } catch (uploadErr: unknown) {
                      if (
                        currentJob.isCancelled ||
                        jobController.signal.aborted ||
                        (uploadErr instanceof Error &&
                          uploadErr.message === 'Download was cancelled')
                      ) {
                        throw uploadErr
                      }

                      if (
                        uploadErr instanceof Error &&
                        uploadErr.message.includes('ENTITY_BOUNDS_INVALID') &&
                        typeof currentCaption !== 'string'
                      ) {
                        warn(
                          'Dump upload encountered ENTITY_BOUNDS_INVALID, retrying with plain text caption',
                          { track_id: trackId },
                        )
                        currentCaption = caption.text
                        continue
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

                      warn('Track upload to dump failed, retrying', {
                        track_id: trackId,
                        attempt: uploadAttempt,
                        max_retries: maxUploadRetries,
                        delay_ms: delayMs,
                        error: errMsg,
                      })

                      await abortableSleep(delayMs, jobController.signal)
                    }
                  }

                  if (currentJob.isCancelled || jobController.signal.aborted) {
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
                      caption: { text: '' },
                      ...(deliveryChatId === chatId
                        ? { replyTo: replyToMessageId }
                        : {}),
                      silent: isMultiTrack,
                    })
                  }

                  activeUploadText = ''
                  currentJob.activeActionText = ''
                  rippedCount++
                  currentJob.rippedCount = rippedCount
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
                } finally {
                  activeUploadText = ''
                  if (existsSync(ripResult.filePath)) {
                    try {
                      unlinkSync(ripResult.filePath)
                    } catch {}
                  }
                  await updateProgress()
                }
              }
            }
          } catch (err) {
            if (
              currentJob.isCancelled ||
              jobController.signal.aborted ||
              (err instanceof Error && err.message === 'Download was cancelled')
            ) {
              return
            }
            throw err
          }
        }

        try {
          await Promise.all([downloadTask(), uploadTask()])
        } finally {
          const remaining = channel.drain()
          for (const item of remaining) {
            if (
              item.status.type === 'ready' &&
              existsSync(item.status.ripResult.filePath)
            ) {
              try {
                unlinkSync(item.status.ripResult.filePath)
              } catch {}
            }
          }
          if (existsSync(jobTempDir)) {
            try {
              rmSync(jobTempDir, { recursive: true, force: true })
            } catch {}
          }
        }

        if (currentJob.isCancelled) {
          return
        }

        await sendFinalSummary()
      },
      {
        signal: jobController.signal,
        onPositionChange: (pos) => {
          currentJob.queuePosition = pos
          const prefix = cachedCount > 0 ? `⚡ ${cachedCount} cached • ` : ''
          updateProgress(`${prefix}⏳ In Queue: Position #${pos}`, true).catch(
            () => {},
          )
        },
        onStart: () => {
          currentJob.queuePosition = 0
          debug('Rip job started from queue', { jobId })
        },
      },
    )
  } catch (err: unknown) {
    if (currentJob.isCancelled || jobController.signal.aborted) {
      return
    }
    error('Rip job encountered an unhandled error in queue', {
      jobId,
      error: err instanceof Error ? err.message : String(err),
    })
  } finally {
    currentJob.completed = true
    activeJobs.delete(jobId)
  }
}
