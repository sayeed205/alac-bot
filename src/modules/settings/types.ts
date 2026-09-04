import type { TelegramClient } from '@mtcute/bun'
import type { Dispatcher } from '@mtcute/dispatcher'

import type { IAuthService } from '@/modules/auth/service.ts'

import type { ISettingsService } from './service.ts'

export type RippingMode = 'live' | 'cache_only' | 'paused'

export interface BotSettings {
  rippingMode: RippingMode
  albumRipEnabled: boolean
  playlistRipEnabled: boolean
  txtRipEnabled: boolean
  multiLinkRipEnabled: boolean
  maxCollectionTracks: number
}

export const DEFAULT_SETTINGS: BotSettings = {
  rippingMode: 'live',
  albumRipEnabled: true,
  playlistRipEnabled: true,
  txtRipEnabled: true,
  multiLinkRipEnabled: true,
  maxCollectionTracks: 50,
}

export interface SettingsCommandContext {
  dp: Dispatcher
  tg: TelegramClient
  service?: ISettingsService
  auth?: IAuthService
}
