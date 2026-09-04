import { BotKeyboard, html, type TelegramClient } from '@mtcute/bun'
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

function parseDynamicHtml(content: string) {
  return html([content] as unknown as TemplateStringsArray)
}

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
    limitButtons,
    [
      BotKeyboard.callback('🔄 Refresh', 'settings:refresh'),
      BotKeyboard.callback('❌ Close', 'settings:close'),
    ],
  ])
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

  return (
    '⚙️ <b>Bot Settings & Operation Controls</b><br/><br/>' +
    `• <b>Engine Mode:</b> ${modeDescriptions[settings.rippingMode]}<br/>` +
    `• <b>Album Ripping:</b> ${settings.albumRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Playlist Ripping:</b> ${settings.playlistRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>.TXT File Ripping:</b> ${settings.txtRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Multi-Link Ripping:</b> ${settings.multiLinkRipEnabled ? '🟢 Enabled' : '🔴 Disabled'}<br/>` +
    `• <b>Max Collection Limit:</b> <code>${limitText}</code><br/><br/>` +
    '<blockquote>💡 <i>Tap buttons below to toggle. Owner requests always bypass these limits.</i></blockquote>'
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

      if (subCommand === 'txt' || subCommand === 'batch_txt') {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('txtRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `.TXT file ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (
        subCommand === 'multilink' ||
        subCommand === 'multi_link' ||
        subCommand === 'multi'
      ) {
        const val = rawValue === 'on' || rawValue === 'true' || rawValue === '1'
        await service.setSetting('multiLinkRipEnabled', val)
        await msg.answerText(
          parseDynamicHtml(
            `Multi-link ripping set to: <b>${val ? 'ON' : 'OFF'}</b>`,
          ),
        )
        return
      }

      if (subCommand === 'limit' || subCommand === 'max_collection') {
        const limit = Number.parseInt(rawValue ?? '50', 10)
        if (!Number.isNaN(limit) && limit >= 0) {
          await service.setMaxCollectionTracks(limit)
          await msg.answerText(
            parseDynamicHtml(
              `Collection limit set to: <b>${limit === 0 ? 'Unlimited' : limit}</b>`,
            ),
          )
          return
        }
        await msg.answerText(
          parseDynamicHtml(
            'Usage: <code>/settings limit &lt;number&gt;</code> (0 for unlimited)',
          ),
        )
        return
      }
    }

    await renderSettingsMessage(tg, service, msg.chat.id, undefined, msg.id)
  })

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
