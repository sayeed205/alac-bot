import type { Audio, Document } from '@mtcute/bun'
import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { debug, error, infoSpan, warn } from '@/utils/logger.ts'

import { executeRipPipeline } from './rip.ts'
import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export interface TrackReport {
  id: string
  trackId: string
  reporterUserId: number
  reporterChatId: number
  reporterName: string
  reason: string
  trackTitle: string
  trackArtist: string
  trackAlbum: string
  dumpMessageId?: number
  timestamp: number
}

// In-memory registry of active reports and rate limiting
export const activeReports = new Map<string, TrackReport>()
export const activeReportsByTrack = new Map<string, string>()
export const userReportTimestamps = new Map<number, number[]>()

const USER_RATE_LIMIT_MAX = 5
const USER_RATE_LIMIT_WINDOW_MS = 60 * 60 * 1000 // 1 hour

/**
 * Checks and records rate limit for a user (sliding 1-hour window).
 */
export function checkUserRateLimit(userId: number): boolean {
  const now = Date.now()
  const timestamps = userReportTimestamps.get(userId) || []
  const recent = timestamps.filter((t) => now - t < USER_RATE_LIMIT_WINDOW_MS)

  if (recent.length >= USER_RATE_LIMIT_MAX) {
    userReportTimestamps.set(userId, recent)
    return false
  }

  recent.push(now)
  userReportTimestamps.set(userId, recent)
  return true
}

/**
 * Clears rate limit and active report state (useful for tests).
 */
export function resetReportState(): void {
  activeReports.clear()
  activeReportsByTrack.clear()
  userReportTimestamps.clear()
}

/**
 * Parses Apple Music track ID from text or URLs.
 */
export function extractTrackIdFromText(text: string): string | null {
  if (!text) return null
  const songUrlMatch = text.match(
    /(?:https?:\/\/)?(?:music|itunes)\.apple\.com\/(?:[a-z]{2}\/)?(?:song|album)\/(?:[^/\s]+\/)?(?:id)?(\d+)(?:\?i=(\d+))?/i,
  )
  if (songUrlMatch) {
    return songUrlMatch[2] || songUrlMatch[1] || null
  }
  const directIdMatch = text.match(/\b(\d{8,11})\b/)
  if (directIdMatch) {
    return directIdMatch[1]
  }
  return null
}

export function buildAdminReportKeyboard(
  trackId: string,
  reportId: string,
): ReturnType<typeof BotKeyboard.inline> {
  return BotKeyboard.inline([
    [
      BotKeyboard.callback(
        '🔄 Re-rip Now (-f)',
        `report:act:rerip:${trackId}:${reportId}`,
      ),
      BotKeyboard.callback(
        '🗑️ Delete Track',
        `report:act:del:${trackId}:${reportId}`,
      ),
    ],
    [BotKeyboard.callback('❌ Dismiss', `report:act:dismiss:${reportId}`)],
  ])
}

export function buildReasonKeyboard(
  trackId: string,
): ReturnType<typeof BotKeyboard.inline> {
  return BotKeyboard.inline([
    [
      BotKeyboard.callback(
        "🔇 Corrupted / Won't Play",
        `report:sub:${trackId}:corrupted`,
      ),
      BotKeyboard.callback(
        '✂️ Incomplete / Cut Off',
        `report:sub:${trackId}:incomplete`,
      ),
    ],
    [
      BotKeyboard.callback(
        '🏷️ Wrong Tags / Metadata',
        `report:sub:${trackId}:metadata`,
      ),
      BotKeyboard.callback(
        '✍️ Other (Custom Note)',
        `report:sub:${trackId}:other`,
      ),
    ],
    [BotKeyboard.callback('❌ Cancel', 'report:cancel')],
  ])
}

export function registerReportCommand(ctx: CommandContext): void {
  const { dp, tg, auth, service } = ctx

  dp.onNewMessage(filters.command(['report', 'issue']), async (msg) => {
    using _ = infoSpan('report_command').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed && !auth.isAdmin(msg.sender.id)) {
      debug('Unauthorized user attempted /report', { user_id: msg.sender.id })
      return
    }

    const reply = await msg.getReplyTo().catch(() => null)
    const rawArgs = msg.command?.length
      ? msg.text.slice(msg.command[0].length + 1).trim()
      : ''

    let targetTrack: Awaited<ReturnType<typeof service.findCachedTrack>> = null
    let customReason = ''

    // 1. Check if user replied to a media message (audio or document)
    if (reply?.media) {
      const media = reply.media as Audio | Document
      const uniqueFileId = media.uniqueFileId
      if (uniqueFileId) {
        targetTrack = await service.findTrackByFileUniqueId(uniqueFileId)
      }

      // If not resolved by uniqueFileId, attempt to parse track ID from reply caption/text
      if (!targetTrack) {
        const replyText = reply.text || ''
        const extractedId = extractTrackIdFromText(replyText)
        if (extractedId) {
          targetTrack = await service.findCachedTrack(extractedId)
        }
      }

      // Any text supplied in the /report command becomes the custom description
      if (rawArgs) {
        customReason = rawArgs
      }
    } else if (rawArgs) {
      // 2. No reply: inspect command arguments (e.g. /report <link|id> [reason])
      const tokens = rawArgs.split(/\s+/)
      const candidateIdOrUrl = tokens[0]
      const extractedId = extractTrackIdFromText(candidateIdOrUrl)

      if (extractedId) {
        targetTrack = await service.findCachedTrack(extractedId)
        customReason = tokens.slice(1).join(' ').trim()
      } else {
        customReason = rawArgs
      }
    }

    // 3. If target track could not be resolved, prompt with clear usage guidance
    if (!targetTrack) {
      await msg.replyText(
        parseDynamicHtml(
          '⚠️ <b>Report a Track Issue</b><br/><br/>' +
            '<blockquote><b>How to report an issue:</b><br/>' +
            '• <b>Reply to any song</b> sent by the bot with <code>/report</code><br/>' +
            '• Or reply with your note: <code>/report &lt;description&gt;</code><br/>' +
            '• Or send: <code>/report &lt;apple_music_link_or_id&gt; [description]</code><br/><br/>' +
            '<i>Example: Reply to a song and type <code>/report cuts off at 2:15</code></i></blockquote>',
        ),
      )
      return
    }

    // 4. Check for duplicate pending report on this track
    if (activeReportsByTrack.has(targetTrack.appleTrackId)) {
      await msg.replyText(
        parseDynamicHtml(
          'ℹ️ <b>Already Under Review</b><br/><br/>' +
            `<blockquote>The track <b>${html.escape(targetTrack.title)}</b> has already been reported and is currently under review by the administrator. Thank you for your report!</blockquote>`,
        ),
      )
      return
    }

    // 5. Rate limit check (max 5 reports per user per hour)
    if (!auth.isAdmin(msg.sender.id) && !checkUserRateLimit(msg.sender.id)) {
      await msg.replyText(
        parseDynamicHtml(
          '⏳ <b>Rate Limit Reached</b><br/><br/>' +
            '<blockquote>You can submit a maximum of 5 reports per hour. Please wait before submitting another report.</blockquote>',
        ),
      )
      return
    }

    // 6. If user gave description in text, dispatch report immediately
    if (customReason) {
      const reportId = Math.random().toString(36).slice(2, 10)
      const reporterName =
        msg.sender.displayName || msg.sender.username || `User ${msg.sender.id}`

      const report: TrackReport = {
        id: reportId,
        trackId: targetTrack.appleTrackId,
        reporterUserId: msg.sender.id,
        reporterChatId: msg.chat.id,
        reporterName,
        reason: customReason,
        trackTitle: targetTrack.title,
        trackArtist: targetTrack.artist,
        trackAlbum: targetTrack.album,
        dumpMessageId: targetTrack.messageId,
        timestamp: Date.now(),
      }

      activeReports.set(reportId, report)
      activeReportsByTrack.set(targetTrack.appleTrackId, reportId)

      // Format dump channel message link if available
      const cleanDumpId = Math.abs(Number(env.DUMP_CHANNEL_ID))
        .toString()
        .replace(/^100/, '')
      const dumpLink = targetTrack.messageId
        ? `<a href="https://t.me/c/${cleanDumpId}/${targetTrack.messageId}">Message #${targetTrack.messageId}</a>`
        : '<i>Not available</i>'

      const adminCard =
        '🚨 <b>New Track Issue Report</b><br/><br/>' +
        `👤 <b>Reported by:</b> <a href="tg://user?id=${msg.sender.id}">${html.escape(reporterName)}</a> (<code>${msg.sender.id}</code>)<br/>` +
        `🎵 <b>Track:</b> <b>${html.escape(targetTrack.title)}</b> - ${html.escape(targetTrack.artist)}<br/>` +
        `💽 <b>Album:</b> ${html.escape(targetTrack.album)}<br/>` +
        `🆔 <b>Apple Track ID:</b> <code>${targetTrack.appleTrackId}</code><br/>` +
        `🔗 <b>Dump Message:</b> ${dumpLink}<br/><br/>` +
        '⚠️ <b>Reported Issue:</b><br/>' +
        `<blockquote>${html.escape(customReason)}</blockquote>`

      await tg.sendText(env.ADMIN_ID, parseDynamicHtml(adminCard), {
        replyMarkup: buildAdminReportKeyboard(
          targetTrack.appleTrackId,
          reportId,
        ),
      })

      await msg.replyText(
        parseDynamicHtml(
          '✅ <b>Report Submitted</b><br/><br/>' +
            `<blockquote>Thank you! Your report for <b>${html.escape(targetTrack.title)}</b> has been delivered to the administrator for review.<br/><br/>` +
            `<b>Reason:</b> <i>${html.escape(customReason)}</i></blockquote>`,
        ),
      )
      return
    }

    // 7. No description in command: present interactive reason picker
    await msg.replyText(
      parseDynamicHtml(
        '⚠️ <b>Report Track Issue</b><br/><br/>' +
          `<blockquote>🎵 <b>Track:</b> <b>${html.escape(targetTrack.title)}</b> - ${html.escape(targetTrack.artist)}<br/>` +
          `💽 <b>Album:</b> ${html.escape(targetTrack.album)}<br/><br/>` +
          'Please choose the issue you experienced:</blockquote>',
      ),
      {
        replyMarkup: buildReasonKeyboard(targetTrack.appleTrackId),
      },
    )
  })

  // Callback query dispatcher for report:* actions
  dp.onCallbackQuery(filters.startsWith('report:'), async (query) => {
    using _ = infoSpan('report_callback').enter()

    const isAuthed = await auth.isAuthorized(query.user.id, query.chat.id)
    if (!isAuthed && !auth.isAdmin(query.user.id)) {
      await query.answer({ text: 'Unauthorized', alert: true })
      return
    }

    const data = query.dataStr || ''
    const parts = data.split(':')
    const actionGroup = parts[1] // 'sub', 'act', 'cancel'

    if (actionGroup === 'cancel') {
      await query.answer({ text: 'Report cancelled' })
      await tg
        .deleteMessagesById(query.chat.id, [query.messageId])
        .catch(() => null)
      return
    }

    // User submission via preset buttons (report:sub:<trackId>:<preset>)
    if (actionGroup === 'sub') {
      const trackId = parts[2]
      const preset = parts[3]

      if (preset === 'other') {
        await query.answer({})
        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(
            '✍️ <b>Custom Report Note</b><br/><br/>' +
              '<blockquote>Please reply to the song with your note:<br/>' +
              '<code>/report &lt;your description here&gt;</code></blockquote>',
          ),
          replyMarkup: BotKeyboard.inline([
            [BotKeyboard.callback('❌ Close', 'report:cancel')],
          ]),
        })
        return
      }

      const presetDescriptions: Record<string, string> = {
        corrupted: "🔇 Corrupted / Won't play",
        incomplete: '✂️ Incomplete / Audio cut off',
        metadata: '🏷️ Wrong metadata / tags / lyrics',
      }

      const reasonText =
        presetDescriptions[preset] || '⚠️ General playback problem'

      if (activeReportsByTrack.has(trackId)) {
        await query.answer({
          text: 'This track is already under review.',
          alert: true,
        })
        await tg
          .deleteMessagesById(query.chat.id, [query.messageId])
          .catch(() => null)
        return
      }

      if (!auth.isAdmin(query.user.id) && !checkUserRateLimit(query.user.id)) {
        await query.answer({
          text: 'Rate limit reached (max 5 reports/hour).',
          alert: true,
        })
        return
      }

      const targetTrack = await service.findCachedTrack(trackId)
      if (!targetTrack) {
        await query.answer({
          text: 'Track not found in database.',
          alert: true,
        })
        return
      }

      const reportId = Math.random().toString(36).slice(2, 10)
      const reporterName =
        query.user.displayName || query.user.username || `User ${query.user.id}`

      const report: TrackReport = {
        id: reportId,
        trackId: targetTrack.appleTrackId,
        reporterUserId: query.user.id,
        reporterChatId: query.chat.id,
        reporterName,
        reason: reasonText,
        trackTitle: targetTrack.title,
        trackArtist: targetTrack.artist,
        trackAlbum: targetTrack.album,
        dumpMessageId: targetTrack.messageId,
        timestamp: Date.now(),
      }

      activeReports.set(reportId, report)
      activeReportsByTrack.set(targetTrack.appleTrackId, reportId)

      const cleanDumpId = Math.abs(Number(env.DUMP_CHANNEL_ID))
        .toString()
        .replace(/^100/, '')
      const dumpLink = targetTrack.messageId
        ? `<a href="https://t.me/c/${cleanDumpId}/${targetTrack.messageId}">Message #${targetTrack.messageId}</a>`
        : '<i>Not available</i>'

      const adminCard =
        '🚨 <b>New Track Issue Report</b><br/><br/>' +
        `👤 <b>Reported by:</b> <a href="tg://user?id=${query.user.id}">${html.escape(reporterName)}</a> (<code>${query.user.id}</code>)<br/>` +
        `🎵 <b>Track:</b> <b>${html.escape(targetTrack.title)}</b> - ${html.escape(targetTrack.artist)}<br/>` +
        `💽 <b>Album:</b> ${html.escape(targetTrack.album)}<br/>` +
        `🆔 <b>Apple Track ID:</b> <code>${targetTrack.appleTrackId}</code><br/>` +
        `🔗 <b>Dump Message:</b> ${dumpLink}<br/><br/>` +
        '⚠️ <b>Reported Issue:</b><br/>' +
        `<blockquote>${html.escape(reasonText)}</blockquote>`

      await tg.sendText(env.ADMIN_ID, parseDynamicHtml(adminCard), {
        replyMarkup: buildAdminReportKeyboard(
          targetTrack.appleTrackId,
          reportId,
        ),
      })

      await query.answer({ text: 'Report submitted successfully!' })
      await tg.editMessage({
        chatId: query.chat.id,
        message: query.messageId,
        text: parseDynamicHtml(
          '✅ <b>Report Submitted</b><br/><br/>' +
            `<blockquote>Thank you! Your report for <b>${html.escape(targetTrack.title)}</b> has been delivered to the administrator.<br/><br/>` +
            `<b>Reason:</b> <i>${html.escape(reasonText)}</i></blockquote>`,
        ),
      })
      return
    }

    // Admin actions (report:act:<action>:<trackId>:<reportId>)
    if (actionGroup === 'act') {
      const isAdmin = auth.isAdmin(query.user.id)
      if (!isAdmin) {
        await query.answer({
          text: '🔒 Access restricted to bot owner.',
          alert: true,
        })
        return
      }

      const adminAction = parts[2] // 'rerip', 'del', 'dismiss'

      if (adminAction === 'dismiss') {
        const reportId = parts[3]
        const report = activeReports.get(reportId)
        if (report) {
          activeReportsByTrack.delete(report.trackId)
          activeReports.delete(reportId)
        }
        await query.answer({ text: 'Report dismissed' })
        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(
            '❌ <b>Report Dismissed</b><br/><br/>' +
              `<blockquote>The report for track <code>${html.escape(report?.trackId || 'N/A')}</code> was dismissed.</blockquote>`,
          ),
        })
        return
      }

      if (adminAction === 'del') {
        const trackId = parts[3]
        const reportId = parts[4]
        const report = activeReports.get(reportId)

        await query.answer({ text: 'Deleting track...' })

        const tr = await service.findCachedTrack(trackId)
        if (tr?.messageId) {
          await tg
            .deleteMessagesById(Number(env.DUMP_CHANNEL_ID), [tr.messageId])
            .catch((err) => {
              warn('Failed to delete dump message on report delete', {
                track_id: trackId,
                error: String(err),
              })
            })
        }

        await service.deleteTrack(trackId)

        if (report) {
          activeReportsByTrack.delete(trackId)
          activeReports.delete(reportId)
          // Courtesy notice to reporter
          await tg
            .sendText(
              report.reporterChatId,
              parseDynamicHtml(
                'ℹ️ <b>Report Update:</b><br/>' +
                  `The reported track <b>${html.escape(report.trackTitle)}</b> has been removed from the database by the administrator.`,
              ),
            )
            .catch(() => null)
        }

        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(
            '🗑️ <b>Track Deleted</b><br/><br/>' +
              `<blockquote>Track <code>${html.escape(trackId)}</code> (<b>${html.escape(report?.trackTitle || 'Track')}</b>) has been deleted from both the database and dump channel.</blockquote>`,
          ),
        })
        return
      }

      if (adminAction === 'rerip') {
        const trackId = parts[3]
        const reportId = parts[4]
        const report = activeReports.get(reportId)

        await query.answer({ text: 'Starting force re-rip...' })

        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(
            '🔄 <b>Re-ripping Track...</b><br/><br/>' +
              `<blockquote>Re-ripping track <code>${html.escape(trackId)}</code> (<b>${html.escape(report?.trackTitle || 'Track')}</b>) in force cache-only mode. Old corrupted dump message will be replaced automatically.</blockquote>`,
          ),
        })

        try {
          await executeRipPipeline(ctx, {
            chatId: query.chat.id,
            userId: query.user.id,
            parsedItems: [{ type: 'track', id: trackId }],
            isForce: true,
            isCacheOnly: true,
          })

          if (report) {
            activeReportsByTrack.delete(trackId)
            activeReports.delete(reportId)
            // Courtesy notice to reporter
            await tg
              .sendText(
                report.reporterChatId,
                parseDynamicHtml(
                  '✅ <b>Report Update:</b><br/>' +
                    `The issue with <b>${html.escape(report.trackTitle)}</b> has been resolved! The track was re-ripped and replaced with a healthy lossless version.`,
                ),
              )
              .catch(() => null)
          }

          await tg.editMessage({
            chatId: query.chat.id,
            message: query.messageId,
            text: parseDynamicHtml(
              '✅ <b>Track Re-ripped Successfully!</b><br/><br/>' +
                `<blockquote>Track <code>${html.escape(trackId)}</code> (<b>${html.escape(report?.trackTitle || 'Track')}</b>) was successfully re-ripped and replaced in the dump channel. Old message was deleted.</blockquote>`,
            ),
          })
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err)
          error('Force re-rip from report failed', {
            track_id: trackId,
            error: errMsg,
          })
          await tg.editMessage({
            chatId: query.chat.id,
            message: query.messageId,
            text: parseDynamicHtml(
              '❌ <b>Re-rip Failed</b><br/><br/>' +
                `<blockquote>Failed to re-rip track <code>${html.escape(trackId)}</code>: <code>${html.escape(errMsg)}</code></blockquote>`,
            ),
            replyMarkup: buildAdminReportKeyboard(trackId, reportId),
          })
        }
        return
      }
    }
  })
}
