import type { Message } from '@mtcute/bun'
import { filters, type MessageContext } from '@mtcute/dispatcher'

import { env } from '@/env.ts'
import { getAppleMusicDeveloperToken } from '@/modules/alac/playlist.ts'
import { settingsService as defaultSettingsService } from '@/modules/settings/service.ts'
import { parseDynamicHtml } from '@/utils/html.ts'
import { debug, error, info, warn } from '@/utils/logger.ts'
import { editMessageSafe, sendTextSafe } from '@/utils/telegram.ts'

import { executeRipPipeline } from './rip.ts'
import type { CommandContext } from './types.ts'

export interface DiscoveredTrackItem {
  id: string
  storefront: string
}

export interface DiscoveredNewTracksResult {
  tracks: DiscoveredTrackItem[]
  trackIds: string[]
  storefrontCounts: Record<string, number>
  cutoffDateString: string
  totalFound: number
}

export interface RunAutoDumpOptions {
  days: number
  triggeredBy: string
  chatId?: number
  userId?: number
  msg?: MessageContext
}

// Global flag to prevent concurrent sweeps
let isAutoDumpRunning = false
let autoDumpTimer: ReturnType<typeof setInterval> | null = null

export function isAutoDumpInProgress(): boolean {
  return isAutoDumpRunning
}

export function resetAutoDumpState(): void {
  isAutoDumpRunning = false
  if (autoDumpTimer) {
    clearInterval(autoDumpTimer)
    autoDumpTimer = null
  }
}

/**
 * Calculates a cutoff date string in YYYY-MM-DD format based on the given days back.
 */
export function getCutoffDateString(days: number): string {
  const cutoff = new Date()
  cutoff.setDate(cutoff.getDate() - Math.max(1, days))
  return cutoff.toISOString().slice(0, 10)
}

/**
 * Discovers newly added tracks across the configured storefronts within the cutoff window.
 * Queries:
 * 1. Curated Apple Music "New Music Daily" editorial playlist (pl.2b0e6e332fdf4b7a91164da3162127b5)
 * 2. Apple Marketing Tools RSS feed for top new albums, resolving song tracks via iTunes lookup.
 */
export async function discoverNewTracks(
  storefronts: string[],
  days: number,
): Promise<DiscoveredNewTracksResult> {
  const cutoffDateString = getCutoffDateString(days)
  const discoveredTracksMap = new Map<string, string>()
  const storefrontCounts: Record<string, number> = {}

  let devToken: string | null = null
  try {
    devToken = await getAppleMusicDeveloperToken()
  } catch (err) {
    warn('Could not acquire Apple Music developer token for autodump', {
      error: String(err),
    })
  }

  for (const sf of storefronts) {
    storefrontCounts[sf] = 0

    // Source 1: New Music Daily editorial playlist (Catalog API)
    if (devToken) {
      try {
        const plUrl = `https://api.music.apple.com/v1/catalog/${sf}/playlists/pl.2b0e6e332fdf4b7a91164da3162127b5?include=tracks`
        const plRes = await fetch(plUrl, {
          headers: {
            Authorization: `Bearer ${devToken}`,
            Origin: 'https://music.apple.com',
          },
          signal: AbortSignal.timeout(10000),
        })

        if (plRes.ok) {
          const plData = (await plRes.json()) as {
            data?: Array<{
              relationships?: {
                tracks?: {
                  data?: Array<{
                    id?: string
                    attributes?: { releaseDate?: string }
                  }>
                }
              }
            }>
          }

          const tracks = plData.data?.[0]?.relationships?.tracks?.data ?? []
          for (const track of tracks) {
            const relDate = track.attributes?.releaseDate || ''
            if (track.id && relDate >= cutoffDateString) {
              if (!discoveredTracksMap.has(track.id)) {
                discoveredTracksMap.set(track.id, sf)
                storefrontCounts[sf] = (storefrontCounts[sf] ?? 0) + 1
              }
            }
          }
        }
      } catch (err) {
        debug('Failed fetching New Music Daily playlist for storefront', {
          storefront: sf,
          error: String(err),
        })
      }
    }

    // Source 2: Apple Marketing Tools Top 50 Albums RSS feed
    try {
      const rssUrl = `https://rss.applemarketingtools.com/api/v2/${sf}/music/most-played/50/albums.json`
      const rssRes = await fetch(rssUrl, {
        signal: AbortSignal.timeout(10000),
      })

      if (rssRes.ok) {
        const rssData = (await rssRes.json()) as {
          feed?: {
            results?: Array<{
              id: string
              releaseDate?: string
            }>
          }
        }

        const albums = rssData.feed?.results ?? []
        for (const album of albums) {
          const relDate = album.releaseDate || ''
          if (album.id && relDate >= cutoffDateString) {
            // Fetch individual tracks in album via iTunes lookup API
            try {
              const itunesUrl = `https://itunes.apple.com/lookup?id=${album.id}&entity=song&country=${sf}`
              const itunesRes = await fetch(itunesUrl, {
                signal: AbortSignal.timeout(8000),
              })

              if (itunesRes.ok) {
                const itunesData = (await itunesRes.json()) as {
                  results?: Array<{
                    wrapperType?: string
                    trackId?: number
                    releaseDate?: string
                  }>
                }

                for (const item of itunesData.results ?? []) {
                  if (item.wrapperType === 'track' && item.trackId) {
                    const trackRelDate = (item.releaseDate || '').slice(0, 10)
                    if (
                      !trackRelDate ||
                      trackRelDate >= cutoffDateString ||
                      relDate >= cutoffDateString
                    ) {
                      const tId = String(item.trackId)
                      if (!discoveredTracksMap.has(tId)) {
                        discoveredTracksMap.set(tId, sf)
                        storefrontCounts[sf] = (storefrontCounts[sf] ?? 0) + 1
                      }
                    }
                  }
                }
              }
            } catch (err) {
              debug('Failed iTunes lookup for album', {
                albumId: album.id,
                error: String(err),
              })
            }
          }
        }
      }
    } catch (err) {
      debug('Failed fetching RSS new albums for storefront', {
        storefront: sf,
        error: String(err),
      })
    }
  }

  const tracks: DiscoveredTrackItem[] = Array.from(
    discoveredTracksMap.entries(),
  ).map(([id, storefront]) => ({ id, storefront }))
  const trackIds = tracks.map((t) => t.id)

  info('Autodump discovery completed', {
    days,
    cutoffDate: cutoffDateString,
    totalFound: tracks.length,
    storefrontCounts,
  })

  return {
    tracks,
    trackIds,
    storefrontCounts,
    cutoffDateString,
    totalFound: tracks.length,
  }
}

/**
 * Runs the full Auto-Dump discovery and download pipeline.
 */
export async function runAutoDumpPipeline(
  ctx: CommandContext,
  options: RunAutoDumpOptions,
): Promise<{ success: boolean; totalFound: number }> {
  const { tg } = ctx
  const settings = ctx.settings ?? defaultSettingsService
  const adminId = Number(env.ADMIN_ID)
  const targetChatId = options.chatId ?? adminId
  const targetUserId = options.userId ?? adminId

  if (isAutoDumpRunning) {
    warn('Auto-dump pipeline already running, skipping new trigger', {
      triggeredBy: options.triggeredBy,
    })
    return { success: false, totalFound: 0 }
  }

  isAutoDumpRunning = true

  const activeStorefronts = settings.getAutoDumpStorefronts()
  const sfFormatted = activeStorefronts.map((s) => s.toUpperCase()).join(', ')

  let statusMsg: Message | undefined
  const initialHtml =
    `🔍 <b>Scanning Apple Music for New Releases...</b><br/><br/>` +
    `<blockquote>• <b>Time Window:</b> Last <code>${options.days}</code> day(s)<br/>` +
    `• <b>Storefronts:</b> <code>${sfFormatted}</code><br/>` +
    `• <b>Triggered By:</b> ${options.triggeredBy}<br/>` +
    `• <b>Mode:</b> Dump Channel Archiver (Cache-Only)</blockquote>`

  try {
    if (options.msg) {
      statusMsg = await options.msg.replyText(parseDynamicHtml(initialHtml))
    } else {
      const res = await sendTextSafe(tg, {
        chatId: targetChatId,
        text: parseDynamicHtml(initialHtml),
        block: true,
        maxWaitSec: 10,
      })
      if (typeof res === 'object' && res) {
        statusMsg = res as Message
      }
    }
  } catch (err) {
    debug('Failed to send initial autodump status message', {
      error: String(err),
    })
  }

  try {
    const discovery = await discoverNewTracks(activeStorefronts, options.days)

    if (discovery.totalFound === 0) {
      const emptyHtml =
        `✨ <b>Auto-Dump Finished: No New Tracks Found</b><br/><br/>` +
        `<blockquote>• <b>Time Window:</b> Last <code>${options.days}</code> day(s)<br/>` +
        `• <b>Cutoff Date:</b> <code>${discovery.cutoffDateString}</code><br/>` +
        `• <b>Storefronts Scanned:</b> <code>${sfFormatted}</code><br/>` +
        `• <b>Discovered:</b> <code>0 tracks</code></blockquote>`

      if (statusMsg) {
        await editMessageSafe(tg, {
          chatId: targetChatId,
          message: statusMsg.id,
          text: parseDynamicHtml(emptyHtml),
        })
      }
      return { success: true, totalFound: 0 }
    }

    const progressHtml =
      `🚀 <b>Discovered ${discovery.totalFound} New Track(s)!</b><br/><br/>` +
      `<blockquote>• <b>Time Window:</b> Last <code>${options.days}</code> day(s)<br/>` +
      `• <b>Cutoff Date:</b> <code>${discovery.cutoffDateString}</code><br/>` +
      `• <b>Storefronts:</b> <code>${sfFormatted}</code><br/>` +
      `• <b>Status:</b> Enqueuing into dump pipeline...</blockquote><br/>` +
      `<i>Archiving lossless ALAC tracks directly to the dump channel. Already cached songs will be skipped instantly.</i>`

    if (statusMsg) {
      await editMessageSafe(tg, {
        chatId: targetChatId,
        message: statusMsg.id,
        text: parseDynamicHtml(progressHtml),
      })
    }

    // Execute through rip pipeline in cache-only dump mode (no cap, skips cached tracks, uploads misses to dump channel)
    await executeRipPipeline(ctx, {
      parsedItems: discovery.tracks.map((t) => ({
        type: 'track' as const,
        id: t.id,
        storefront: t.storefront,
      })),
      userId: targetUserId,
      chatId: targetChatId,
      statusMessageToReuse: statusMsg,
      isForce: false,
      isCacheOnly: true,
    })

    // Notify admin via DM with a final summary card
    const sfBreakdown = Object.entries(discovery.storefrontCounts)
      .map(
        ([sf, count]) => `• <b>${sf.toUpperCase()}:</b> <code>${count}</code>`,
      )
      .join('<br/>')

    const summaryHtml =
      `📦 <b>Auto-Dump Run Completed</b><br/><br/>` +
      `<blockquote>• <b>Total Discovered:</b> <code>${discovery.totalFound} tracks</code><br/>` +
      `• <b>Time Window:</b> Last <code>${options.days}</code> day(s)<br/>` +
      `• <b>Trigger:</b> ${options.triggeredBy}<br/><br/>` +
      `<b>Storefront Breakdown:</b><br/>${sfBreakdown}</blockquote><br/>` +
      `<blockquote>💡 <i>All new tracks have been securely archived to the dump channel.</i></blockquote>`

    await sendTextSafe(tg, {
      chatId: adminId,
      text: parseDynamicHtml(summaryHtml),
      block: true,
      maxWaitSec: 10,
    })

    return { success: true, totalFound: discovery.totalFound }
  } catch (err) {
    error('Auto-dump pipeline execution failed', { error: String(err) })
    if (statusMsg) {
      await editMessageSafe(tg, {
        chatId: targetChatId,
        message: statusMsg.id,
        text: parseDynamicHtml(
          `❌ <b>Auto-Dump Failed:</b> <code>${String(err)}</code>`,
        ),
      })
    }
    return { success: false, totalFound: 0 }
  } finally {
    isAutoDumpRunning = false
  }
}

/**
 * Registers the on-demand `/dumpnew [days]` and `/autodump [days]` commands (Admin only).
 */
export function registerAutoDumpCommand(ctx: CommandContext): void {
  const { dp, auth } = ctx

  dp.onNewMessage(filters.command(['dumpnew', 'autodump']), async (msg) => {
    if (!auth.isAdmin(msg.sender.id)) {
      debug('Non-admin attempted autodump command', {
        user_id: msg.sender.id,
      })
      return
    }

    const textTokens = msg.text.trim().split(/\s+/)
    let days = 1
    if (textTokens[1]) {
      const parsedDays = Number.parseInt(textTokens[1], 10)
      if (!Number.isNaN(parsedDays) && parsedDays > 0) {
        days = parsedDays
      }
    }

    if (isAutoDumpRunning) {
      await msg.replyText(
        parseDynamicHtml(
          '⏳ <b>Auto-Dump is already in progress!</b> Please wait for the current sweep to finish.',
        ),
      )
      return
    }

    await runAutoDumpPipeline(ctx, {
      days,
      triggeredBy: `Admin Command (/dumpnew ${days})`,
      chatId: msg.chat.id,
      userId: msg.sender.id,
      msg,
    })
  })
}

/**
 * Starts the 24-hour background scheduler for auto-dump.
 */
export function startAutoDumpScheduler(ctx: CommandContext): () => void {
  const settings = ctx.settings ?? defaultSettingsService
  const TWENTY_FOUR_HOURS_MS = 24 * 60 * 60 * 1000

  info('Starting 24h auto-dump scheduler')

  autoDumpTimer = setInterval(async () => {
    if (!settings.isAutoDumpEnabled()) {
      debug('Auto-dump scheduler tick skipped: disabled in settings')
      return
    }

    try {
      info('Executing scheduled daily auto-dump sweep')
      await runAutoDumpPipeline(ctx, {
        days: 1,
        triggeredBy: 'Daily 24h Scheduler',
      })
    } catch (err) {
      error('Scheduled auto-dump sweep encountered error', {
        error: String(err),
      })
    }
  }, TWENTY_FOUR_HOURS_MS)

  return () => {
    if (autoDumpTimer) {
      clearInterval(autoDumpTimer)
      autoDumpTimer = null
      info('Stopped 24h auto-dump scheduler')
    }
  }
}
