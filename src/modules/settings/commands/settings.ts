import { BotKeyboard, type TelegramClient } from '@mtcute/bun'
import { type Dispatcher, filters } from '@mtcute/dispatcher'

import { authService, type IAuthService } from '@/modules/auth/service.ts'
import {
  type ISettingsService,
  settingsService,
} from '@/modules/settings/service.ts'
import type {
  BotSettings,
  SettingsCommandContext,
} from '@/modules/settings/types.ts'
import { parseDynamicHtml } from '@/utils/html.ts'

export const POPULAR_STOREFRONTS = [
  'us',
  'gb',
  'jp',
  'in',
  'ca',
  'au',
  'de',
  'fr',
]

export function buildSettingsKeyboard(settings: BotSettings) {
  const modeLabels = {
    live: 'Mode: 🟢 Live Ripping',
    cache_only: 'Mode: 🟡 Cache Only',
    paused: 'Mode: 🔴 Fully Paused',
  }

  const limitPresets = [25, 50, 100, 0]
  const limitButtons = limitPresets.map((preset) => {
    const isSelected = settings.maxCollectionTracks === preset
    const label = preset === 0 ? 'Unlimited' : String(preset)
    const text = isSelected ? `✅ ${label}` : label
    return BotKeyboard.callback(text, `settings:limit:${preset}`)
  })

  return BotKeyboard.inline([
    [BotKeyboard.callback(modeLabels[settings.rippingMode], 'settings:mode')],
    [
      BotKeyboard.callback(
        settings.albumRipEnabled ? 'Albums: 🟢 ON' : 'Albums: 🔴 OFF',
        'settings:album',
      ),
      BotKeyboard.callback(
        settings.playlistRipEnabled ? 'Playlists: 🟢 ON' : 'Playlists: 🔴 OFF',
        'settings:playlist',
      ),
      BotKeyboard.callback(
        settings.artistRipEnabled ? 'Artists: 🟢 ON' : 'Artists: 🔴 OFF',
        'settings:artist',
      ),
    ],
    [
      BotKeyboard.callback(
        settings.txtRipEnabled ? '.TXT Batch: 🟢 ON' : '.TXT Batch: 🔴 OFF',
        'settings:txt',
      ),
      BotKeyboard.callback(
        settings.multiLinkRipEnabled
          ? 'Multi-Link: 🟢 ON'
          : 'Multi-Link: 🔴 OFF',
        'settings:multilink',
      ),
    ],
    [
      BotKeyboard.callback(
        settings.autoDumpEnabled ? 'Auto-Dump: 🟢 ON' : 'Auto-Dump: 🔴 OFF',
        'settings:autodump',
      ),
      BotKeyboard.callback(
        `🌐 Storefronts (${settings.autoDumpStorefronts.length})`,
        'settings:sf_menu',
      ),
    ],
    limitButtons,
    [
      BotKeyboard.callback('🔄 Refresh', 'settings:refresh'),
      BotKeyboard.callback('❌ Close', 'settings:close'),
    ],
  ])
}

export function buildStorefrontsKeyboard(settings: BotSettings) {
  const currentSfs = new Set(
    settings.autoDumpStorefronts.map((s) => s.toLowerCase()),
  )
  const buttons: ReturnType<typeof BotKeyboard.callback>[][] = []

  // 4 buttons per row
  for (let i = 0; i < POPULAR_STOREFRONTS.length; i += 4) {
    const row = POPULAR_STOREFRONTS.slice(i, i + 4).map((sf) => {
      const active = currentSfs.has(sf)
      const label = active ? `✅ ${sf.toUpperCase()}` : sf.toUpperCase()
      return BotKeyboard.callback(label, `settings:sf:toggle:${sf}`)
    })
    buttons.push(row)
  }

  buttons.push([
    BotKeyboard.callback('🔙 Back to Settings', 'settings:refresh'),
    BotKeyboard.callback('❌ Close', 'settings:close'),
  ])

  return BotKeyboard.inline(buttons)
}

export function renderSettingsText(settings: BotSettings): string {
  const modeDescriptions = {
    live: '🟢 <b>Live Ripping</b> (Normal operation: cache hits + live decryption)',
    cache_only:
      '🟡 <b>Cache Only</b> (Serves cached songs; live decryption blocked)',
    paused:
      '🔴 <b>Paused</b> (All ripping commands suspended for regular users)',
  }

  const limitText =
    settings.maxCollectionTracks === 0
      ? 'Unlimited'
      : `${settings.maxCollectionTracks} tracks`

  const sfList = settings.autoDumpStorefronts
    .map((s) => s.toUpperCase())
    .join(', ')

  return (
    '⚙️ <b>Bot Settings & Operation Controls</b><br/><br/>' +
    `• <b>Engine Mode:</b> ${modeDescriptions[settings.rippingMode]}<br/>` +
    `• <b>Album Ripping:</b> ${settings.albumRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Playlist Ripping:</b> ${settings.playlistRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Artist Ripping:</b> ${settings.artistRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>.TXT File Ripping:</b> ${settings.txtRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Multi-Link Ripping:</b> ${settings.multiLinkRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Auto-Dump New Music:</b> ${settings.autoDumpEnabled ? '🟢 Enabled (Daily)' : '🔴 Disabled'}<br/>` +
    `• <b>Auto-Dump Storefronts:</b> <code>${sfList}</code><br/>` +
    `• <b>Max Collection Limit:</b> <code>${limitText}</code><br/><br/>` +
    '<blockquote>💡 <i>Tap buttons below to toggle. Owner requests always bypass these limits.</i></blockquote>'
  )
}

export function renderStorefrontsText(settings: BotSettings): string {
  const sfList = settings.autoDumpStorefronts
    .map((s) => s.toUpperCase())
    .join(', ')

  return (
    '🌐 <b>Auto-Dump Storefront Configuration</b><br/><br/>' +
    `• <b>Active Storefronts:</b> <code>${sfList}</code><br/><br/>` +
    'Tap a country below to toggle it on or off for the daily new music auto-dump.<br/>' +
    '<i>You can also use:</i> <code>/settings storefronts add &lt;code&gt;</code>'
  )
}

export async function renderSettingsMessage(
  tg: TelegramClient,
  service: ISettingsService,
  chatId: number,
  messageId?: number,
  replyToMessageId?: number,
) {
  const currentSettings = service.getSettings()
  const text = parseDynamicHtml(renderSettingsText(currentSettings))
  const keyboard = buildSettingsKeyboard(currentSettings)

  if (messageId) {
    await tg
      .editMessage({
        chatId,
        message: messageId,
        text,
        replyMarkup: keyboard,
      })
      .catch((err: unknown) => {
        if (
          err &&
          typeof err === 'object' &&
          'message' in err &&
          String((err as { message: unknown }).message).includes('NOT_MODIFIED')
        ) {
          return
        }
      })
  } else {
    await tg.sendText(chatId, text, {
      replyMarkup: keyboard,
      replyTo: replyToMessageId,
    })
  }
}

export async function renderStorefrontsMessage(
  tg: TelegramClient,
  service: ISettingsService,
  chatId: number,
  messageId: number,
) {
  const currentSettings = service.getSettings()
  const text = parseDynamicHtml(renderStorefrontsText(currentSettings))
  const keyboard = buildStorefrontsKeyboard(currentSettings)

  await tg
    .editMessage({
      chatId,
      message: messageId,
      text,
      replyMarkup: keyboard,
    })
    .catch(() => null)
}

export function registerSettingsCommands(ctx: SettingsCommandContext): void
export function registerSettingsCommands(
  dp: Dispatcher,
  tg: TelegramClient,
  service?: ISettingsService,
  auth?: IAuthService,
): void
export function registerSettingsCommands(
  dpOrCtx: Dispatcher | SettingsCommandContext,
  tgClient?: TelegramClient,
  customService?: ISettingsService,
  customAuth?: IAuthService,
): void {
  const ctx: SettingsCommandContext =
    'dp' in dpOrCtx
      ? dpOrCtx
      : {
          dp: dpOrCtx,
          tg: tgClient as TelegramClient,
          service: customService,
          auth: customAuth,
        }

  const { dp, tg } = ctx
  const service = ctx.service ?? settingsService
  const auth = ctx.auth ?? authService

  dp.onNewMessage(filters.command('settings'), async (msg) => {
    if (!auth.isAdmin(msg.sender.id)) {
      return
    }

    const textParts = msg.text.trim().split(/\s+/)
    if (textParts.length >= 3) {
      const subCommand = textParts[1]?.toLowerCase()
      const rawValue = textParts[2]?.toLowerCase()

      if (subCommand === 'mode') {
        if (
          rawValue === 'live' ||
          rawValue === 'cache_only' ||
          rawValue === 'paused'
        ) {
          await service.setSetting('rippingMode', rawValue)
          await msg.answerText(
            parseDynamicHtml(`Engine mode set to: <b>${rawValue}</b>`),
          )
          return
        }
        await msg.answerText(
          parseDynamicHtml(
            'Usage: <code>/settings mode &lt;live|cache_only|paused&gt;</code>',
          ),
        )
        return
      }

      if (subCommand === 'album') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('albumRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Album ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'playlist') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('playlistRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Playlist ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'artist') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('artistRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Artist ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'txt' || subCommand === 'batch_txt') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('txtRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `.TXT batch ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'multilink' || subCommand === 'multi_link') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('multiLinkRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Multi-link ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'autodump' || subCommand === 'auto_dump') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('autoDumpEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Auto-dump new music set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (
        subCommand === 'storefronts' ||
        subCommand === 'storefront' ||
        subCommand === 'sf'
      ) {
        const action = rawValue
        const targetSf = textParts[3]?.toLowerCase()

        if (action === 'add' && targetSf) {
          const list = await service.addAutoDumpStorefront(targetSf)
          await msg.answerText(
            parseDynamicHtml(
              `Added <b>${targetSf.toUpperCase()}</b>. Storefronts: <code>${list.map((s) => s.toUpperCase()).join(', ')}</code>`,
            ),
          )
          return
        }

        if (
          (action === 'remove' || action === 'rm' || action === 'del') &&
          targetSf
        ) {
          const list = await service.removeAutoDumpStorefront(targetSf)
          await msg.answerText(
            parseDynamicHtml(
              `Removed <b>${targetSf.toUpperCase()}</b>. Storefronts: <code>${list.map((s) => s.toUpperCase()).join(', ')}</code>`,
            ),
          )
          return
        }

        if (action === 'set' && targetSf) {
          const targets = textParts
            .slice(3)
            .join(',')
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean)
          const list = await service.setAutoDumpStorefronts(targets)
          await msg.answerText(
            parseDynamicHtml(
              `Storefronts set to: <code>${list.map((s) => s.toUpperCase()).join(', ')}</code>`,
            ),
          )
          return
        }

        await msg.answerText(
          parseDynamicHtml(
            'Usage: <code>/settings storefronts &lt;add|rm|set&gt; &lt;code&gt;</code>',
          ),
        )
        return
      }

      if (subCommand === 'limit') {
        const num = Number.parseInt(rawValue, 10)
        if (!Number.isNaN(num) && num >= 0) {
          const updated = await service.setMaxCollectionTracks(num)
          await msg.answerText(
            parseDynamicHtml(
              `Max collection limit set to: <b>${updated === 0 ? 'Unlimited' : updated}</b>`,
            ),
          )
          return
        }
        await msg.answerText(
          parseDynamicHtml(
            'Usage: <code>/settings limit &lt;number (0 for unlimited)&gt;</code>',
          ),
        )
        return
      }
    }

    await renderSettingsMessage(tg, service, msg.chat.id, undefined, msg.id)
  })

  // Callback query handling for settings menu
  dp.onCallbackQuery(filters.startsWith('settings:'), async (query) => {
    const data = query.dataStr
    if (!data) return

    if (!auth.isAdmin(query.user.id)) {
      await query.answer({ text: 'Unauthorized. Owner only.', alert: true })
      return
    }

    if (data === 'settings:close') {
      await query.answer({})
      await tg
        .deleteMessagesById(query.chat.id, [query.messageId])
        .catch(() => null)
      return
    }

    if (data === 'settings:refresh') {
      await query.answer({ text: 'Settings refreshed' })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:sf_menu') {
      await query.answer({})
      await renderStorefrontsMessage(
        tg,
        service,
        query.chat.id,
        query.messageId,
      )
      return
    }

    if (data.startsWith('settings:sf:toggle:')) {
      const sf = data.split(':')[3]?.toLowerCase()
      if (sf) {
        const current = service.getAutoDumpStorefronts()
        if (current.includes(sf)) {
          await service.removeAutoDumpStorefront(sf)
          await query.answer({ text: `Removed ${sf.toUpperCase()}` })
        } else {
          await service.addAutoDumpStorefront(sf)
          await query.answer({ text: `Added ${sf.toUpperCase()}` })
        }
        await renderStorefrontsMessage(
          tg,
          service,
          query.chat.id,
          query.messageId,
        )
      }
      return
    }

    if (data === 'settings:mode') {
      const newMode = await service.cycleRippingMode()
      const labels = {
        live: 'Mode: Live Ripping',
        cache_only: 'Mode: Cache Only',
        paused: 'Mode: Fully Paused',
      }
      await query.answer({ text: labels[newMode] })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:album') {
      const enabled = await service.toggleAlbumRip()
      await query.answer({
        text: `Album ripping: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:playlist') {
      const enabled = await service.togglePlaylistRip()
      await query.answer({
        text: `Playlist ripping: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:artist') {
      const enabled = await service.toggleArtistRip()
      await query.answer({
        text: `Artist ripping: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:txt') {
      const enabled = await service.toggleTxtRip()
      await query.answer({
        text: `.TXT batch ripping: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:multilink') {
      const enabled = await service.toggleMultiLinkRip()
      await query.answer({
        text: `Multi-link ripping: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data === 'settings:autodump') {
      const enabled = await service.toggleAutoDump()
      await query.answer({
        text: `Auto-dump: ${enabled ? 'ON' : 'OFF'}`,
      })
      await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      return
    }

    if (data.startsWith('settings:limit:')) {
      const limitStr = data.split(':')[2]
      const limit = Number.parseInt(limitStr || '50', 10)
      if (!Number.isNaN(limit) && limit >= 0) {
        await service.setMaxCollectionTracks(limit)
        await query.answer({
          text: `Collection limit: ${limit === 0 ? 'Unlimited' : `${limit} tracks`}`,
        })
        await renderSettingsMessage(tg, service, query.chat.id, query.messageId)
      }
    }
  })
}
