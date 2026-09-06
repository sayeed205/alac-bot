import { BotKeyboard, html } from '@mtcute/bun'
import { filters } from '@mtcute/dispatcher'

import { fetchAlbumTracks } from '@/modules/alac/itunes.ts'
import { debug, error, infoSpan } from '@/utils/logger.ts'

import { executeRipPipeline } from './rip.ts'
import type { CommandContext } from './types.ts'
import { parseDynamicHtml } from './types.ts'

export interface RandomAlbumCandidate {
  id: string
  title: string
  artist: string
  url: string
  artworkUrl?: string
  releaseDate?: string
  genre?: string
  trackCount?: number
  storefront: string
}

export const WILD_SEEDS = [
  'future',
  'midnight',
  'electric',
  'dream',
  'sunset',
  'horizon',
  'velvet',
  'echo',
  'shadow',
  'crystal',
  'neon',
  'ocean',
  'silver',
  'aurora',
  'cosmic',
  'paradise',
  'vintage',
  'solitude',
  'infinite',
  'rhythm',
  'harmony',
  'odyssey',
  'mirage',
  'serenade',
  'astral',
  'stellar',
  'cascade',
  'monochrome',
  'sanctuary',
  'voyage',
  'illusions',
  'phantom',
  'solstice',
  'vortex',
  'genesis',
  'celestial',
  'timeless',
  'radiant',
  'nostalgia',
  'euphoria',
  'zenith',
  'spectrum',
  'whisper',
  'pulse',
  'nebula',
  'labyrinth',
  'reverie',
  'resonance',
  'destiny',
  'memory',
]

export const SOURCE_LABELS: Record<string, string> = {
  charts: '🏆 Top Charts',
  wild: '🎲 Wild Search',
  rock: '🎸 Rock',
  hiphop: '🎤 Hip-Hop',
  pop: '⚡ Pop',
  electronic: '🎹 Electronic',
  jazz: '🎷 Jazz',
  indie: '💿 Indie',
}

export async function fetchChartsAlbum(
  storefront = 'us',
): Promise<RandomAlbumCandidate> {
  const sf = storefront.toLowerCase()
  const url = `https://rss.marketingtools.apple.com/api/v2/${encodeURIComponent(sf)}/music/most-played/50/albums.json`
  const res = await fetch(url, { headers: { 'User-Agent': 'Mozilla/5.0' } })
  if (!res.ok) {
    throw new Error(`Failed to fetch Apple Music charts (HTTP ${res.status})`)
  }
  const data = (await res.json()) as {
    feed?: {
      results?: Array<{
        id: string
        name: string
        artistName: string
        url: string
        artworkUrl100?: string
        releaseDate?: string
        genres?: Array<{ name: string }>
      }>
    }
  }

  const results = data.feed?.results ?? []
  if (results.length === 0) {
    throw new Error('No albums found in Apple Music charts')
  }

  const pick = results[Math.floor(Math.random() * results.length)]
  return {
    id: pick.id,
    title: pick.name,
    artist: pick.artistName,
    url: pick.url,
    artworkUrl: pick.artworkUrl100,
    releaseDate: pick.releaseDate,
    genre: pick.genres?.[0]?.name,
    storefront: sf,
  }
}

export async function fetchSearchAlbum(
  query: string,
  storefront = 'us',
): Promise<RandomAlbumCandidate> {
  const sf = storefront.toLowerCase()
  const url = `https://itunes.apple.com/search?term=${encodeURIComponent(query)}&entity=album&limit=50&country=${encodeURIComponent(sf)}`
  const res = await fetch(url, { headers: { 'User-Agent': 'Mozilla/5.0' } })
  if (!res.ok) {
    throw new Error(`Failed to search iTunes catalog (HTTP ${res.status})`)
  }
  const data = (await res.json()) as {
    results?: Array<{
      collectionId?: number
      collectionName?: string
      artistName?: string
      collectionViewUrl?: string
      artworkUrl100?: string
      releaseDate?: string
      primaryGenreName?: string
      trackCount?: number
    }>
  }

  const results = (data.results ?? []).filter((item) => item.collectionId)
  if (results.length === 0) {
    throw new Error(`No albums found matching "${query}" on storefront ${sf}`)
  }

  const pick = results[Math.floor(Math.random() * results.length)]
  return {
    id: String(pick.collectionId),
    title: pick.collectionName || 'Unknown Album',
    artist: pick.artistName || 'Unknown Artist',
    url:
      pick.collectionViewUrl ||
      `https://music.apple.com/${sf}/album/${pick.collectionId}`,
    artworkUrl: pick.artworkUrl100,
    releaseDate: pick.releaseDate,
    genre: pick.primaryGenreName,
    trackCount: pick.trackCount,
    storefront: sf,
  }
}

export async function fetchWildAlbum(
  storefront = 'us',
): Promise<RandomAlbumCandidate> {
  const seed = WILD_SEEDS[Math.floor(Math.random() * WILD_SEEDS.length)]
  return fetchSearchAlbum(seed, storefront)
}

export async function fetchCandidateBySource(
  source: string,
  storefront = 'us',
): Promise<RandomAlbumCandidate> {
  switch (source.toLowerCase()) {
    case 'charts':
    case 'top':
      return fetchChartsAlbum(storefront)
    case 'wild':
    case 'random':
      return fetchWildAlbum(storefront)
    case 'rock':
      return fetchSearchAlbum('rock album', storefront)
    case 'hiphop':
    case 'rap':
      return fetchSearchAlbum('hip hop album', storefront)
    case 'pop':
      return fetchSearchAlbum('pop album', storefront)
    case 'electronic':
    case 'edm':
      return fetchSearchAlbum('electronic album', storefront)
    case 'jazz':
      return fetchSearchAlbum('jazz album', storefront)
    case 'indie':
      return fetchSearchAlbum('indie album', storefront)
    default:
      return fetchSearchAlbum(source, storefront)
  }
}

export function buildSourcesKeyboard() {
  return BotKeyboard.inline([
    [
      BotKeyboard.callback('🏆 Top Charts', 'random:src:charts'),
      BotKeyboard.callback('🎲 Wild Search', 'random:src:wild'),
    ],
    [
      BotKeyboard.callback('🎸 Rock', 'random:src:rock'),
      BotKeyboard.callback('🎤 Hip-Hop', 'random:src:hiphop'),
      BotKeyboard.callback('⚡ Pop', 'random:src:pop'),
    ],
    [
      BotKeyboard.callback('🎹 Electronic', 'random:src:electronic'),
      BotKeyboard.callback('🎷 Jazz', 'random:src:jazz'),
      BotKeyboard.callback('💿 Indie', 'random:src:indie'),
    ],
    [BotKeyboard.callback('❌ Close', 'random:close')],
  ])
}

export function buildPreviewKeyboard(
  albumId: string,
  storefront: string,
  source: string,
) {
  return BotKeyboard.inline([
    [
      BotKeyboard.callback(
        '🚀 Start Dump',
        `random:dump:${albumId}:${storefront}`,
      ),
    ],
    [
      BotKeyboard.callback(
        '🎲 Re-roll',
        `random:reroll:${source}:${storefront}`,
      ),
      BotKeyboard.callback('⬅️ Sources', 'random:menu'),
    ],
    [BotKeyboard.callback('❌ Cancel', 'random:close')],
  ])
}

export function buildSourcesMenuText(): string {
  return (
    '🎲 <b>Apple Music Random Album Explorer</b> (Admin)<br/><br/>' +
    '<blockquote>Select a discovery source below to pick a random album to dump into your cache channel:</blockquote>'
  )
}

export function buildPreviewText(candidate: RandomAlbumCandidate): string {
  const releaseDateStr = candidate.releaseDate
    ? candidate.releaseDate.split('T')[0]
    : 'Unknown'
  const trackCountStr = candidate.trackCount
    ? `${candidate.trackCount} tracks`
    : 'Full Album'

  return (
    '🎲 <b>Random Album Picked!</b><br/><br/>' +
    `💿 <b>Album:</b> ${html.escape(candidate.title)}<br/>` +
    `👤 <b>Artist:</b> ${html.escape(candidate.artist)}<br/>` +
    `🎵 <b>Tracks:</b> <code>${html.escape(trackCountStr)}</code><br/>` +
    `📅 <b>Released:</b> <code>${html.escape(releaseDateStr)}</code><br/>` +
    `🏷 <b>Genre:</b> <code>${html.escape(candidate.genre || 'Music')}</code><br/>` +
    `🌍 <b>Storefront:</b> <code>${html.escape(candidate.storefront.toUpperCase())}</code><br/><br/>` +
    `🔗 <a href="${html.escape(candidate.url)}">Open in Apple Music</a>`
  )
}

export function registerRandomCommand(ctx: CommandContext): void {
  const { dp, tg, auth } = ctx

  dp.onNewMessage(filters.command('random'), async (msg) => {
    using _ = infoSpan('random_cmd').enter()

    const isAuthed = await auth.isAuthorized(msg.sender.id, msg.chat.id)
    if (!isAuthed) return

    const isAdmin = auth.isAdmin(msg.sender.id)
    if (!isAdmin) {
      debug('Non-admin requested /random command', { user_id: msg.sender.id })
      await msg.replyText(
        parseDynamicHtml(
          '🔒 <b>Access Restricted:</b> Random album discovery is restricted to the bot owner.',
        ),
      )
      return
    }

    const tokens = msg.text.trim().split(/\s+/).slice(1)
    const sourceArg = tokens[0]?.toLowerCase()
    const storefrontArg = tokens[1]?.toLowerCase() || 'us'

    if (!sourceArg) {
      await msg.replyText(parseDynamicHtml(buildSourcesMenuText()), {
        replyMarkup: buildSourcesKeyboard(),
      })
      return
    }

    // Direct argument provided (e.g. /random charts or /random wild)
    const loadingMsg = await msg.replyText(
      parseDynamicHtml(
        `🎲 <i>Discovering random album from ${html.escape(sourceArg)} (${html.escape(storefrontArg.toUpperCase())})...</i>`,
      ),
    )

    try {
      const candidate = await fetchCandidateBySource(sourceArg, storefrontArg)
      try {
        const fullAlbum = await fetchAlbumTracks(
          candidate.id,
          candidate.storefront,
        )
        candidate.trackCount = fullAlbum.tracks.length
        candidate.title = fullAlbum.album.title || candidate.title
        candidate.artist = fullAlbum.album.artist || candidate.artist
        candidate.genre = fullAlbum.album.genre || candidate.genre
        candidate.releaseDate =
          fullAlbum.album.releaseDate || candidate.releaseDate
      } catch (_e) {
        // Fallback to initial candidate metadata
      }

      await tg.editMessage({
        chatId: msg.chat.id,
        message: loadingMsg.id,
        text: parseDynamicHtml(buildPreviewText(candidate)),
        replyMarkup: buildPreviewKeyboard(
          candidate.id,
          candidate.storefront,
          sourceArg,
        ),
      })
    } catch (err: unknown) {
      const errMsg = err instanceof Error ? err.message : String(err)
      error('Failed to resolve random album via command', {
        source: sourceArg,
        error: errMsg,
      })
      await tg.editMessage({
        chatId: msg.chat.id,
        message: loadingMsg.id,
        text: parseDynamicHtml(
          `⚠️ <b>Failed to pick random album:</b> <code>${html.escape(errMsg)}</code>`,
        ),
      })
    }
  })

  // Callback query dispatcher for random:* actions
  dp.onCallbackQuery(filters.startsWith('random:'), async (query) => {
    using _ = infoSpan('random_callback').enter()

    const isAuthed = await auth.isAuthorized(query.user.id, query.chat.id)
    if (!isAuthed) {
      await query.answer({ text: 'Unauthorized', alert: true })
      return
    }

    const isAdmin = auth.isAdmin(query.user.id)
    if (!isAdmin) {
      await query.answer({
        text: '🔒 Access restricted to bot owner.',
        alert: true,
      })
      return
    }

    const data = query.dataStr || ''
    const parts = data.split(':')
    const action = parts[1]

    if (action === 'close') {
      await query.answer({})
      await tg
        .deleteMessagesById(query.chat.id, [query.messageId])
        .catch(() => null)
      return
    }

    if (action === 'menu') {
      await query.answer({})
      await tg.editMessage({
        chatId: query.chat.id,
        message: query.messageId,
        text: parseDynamicHtml(buildSourcesMenuText()),
        replyMarkup: buildSourcesKeyboard(),
      })
      return
    }

    if (action === 'src' || action === 'reroll') {
      const source = parts[2] || 'wild'
      const storefront = parts[3] || 'us'
      await query.answer({ text: '🎲 Discovering random album...' })

      await tg.editMessage({
        chatId: query.chat.id,
        message: query.messageId,
        text: parseDynamicHtml(
          `🔄 <i>Discovering random album from ${html.escape(SOURCE_LABELS[source] || source)}...</i>`,
        ),
      })

      try {
        const candidate = await fetchCandidateBySource(source, storefront)
        try {
          const fullAlbum = await fetchAlbumTracks(
            candidate.id,
            candidate.storefront,
          )
          candidate.trackCount = fullAlbum.tracks.length
          candidate.title = fullAlbum.album.title || candidate.title
          candidate.artist = fullAlbum.album.artist || candidate.artist
          candidate.genre = fullAlbum.album.genre || candidate.genre
          candidate.releaseDate =
            fullAlbum.album.releaseDate || candidate.releaseDate
        } catch (_e) {
          // Fallback to basic candidate metadata
        }

        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(buildPreviewText(candidate)),
          replyMarkup: buildPreviewKeyboard(
            candidate.id,
            candidate.storefront,
            source,
          ),
        })
      } catch (err: unknown) {
        const errMsg = err instanceof Error ? err.message : String(err)
        error('Failed to pick random album on callback', {
          source,
          error: errMsg,
        })
        await tg.editMessage({
          chatId: query.chat.id,
          message: query.messageId,
          text: parseDynamicHtml(
            `⚠️ <b>Failed to discover album:</b> <code>${html.escape(errMsg)}</code>`,
          ),
          replyMarkup: BotKeyboard.inline([
            [
              BotKeyboard.callback(
                '🔄 Try Again',
                `random:reroll:${source}:${storefront}`,
              ),
              BotKeyboard.callback('⬅️ Sources', 'random:menu'),
            ],
            [BotKeyboard.callback('❌ Close', 'random:close')],
          ]),
        })
      }
      return
    }

    if (action === 'dump') {
      const albumId = parts[2]
      const storefront = parts[3] || 'us'

      await query.answer({ text: '🚀 Queuing album dump...' })
      await tg
        .deleteMessagesById(query.chat.id, [query.messageId])
        .catch(() => null)

      // Delegate to battle-tested rip/dump pipeline in cache-only mode
      await executeRipPipeline(ctx, {
        chatId: query.chat.id,
        userId: query.user.id,
        parsedItems: [
          {
            type: 'album',
            id: albumId,
            storefront,
          },
        ],
        isCacheOnly: true,
        singleStorefront: storefront,
      })
    }
  })
}
