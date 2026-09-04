import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { settings } from '@/db/schema.ts'
import { debug, error, info } from '@/utils/logger.ts'

import {
  type BotSettings,
  DEFAULT_SETTINGS,
  type RippingMode,
} from './types.ts'

export interface ISettingsService {
  init(): Promise<void>
  getSettings(): BotSettings
  getRippingMode(): RippingMode
  isAlbumRipEnabled(): boolean
  isPlaylistRipEnabled(): boolean
  isTxtRipEnabled(): boolean
  isMultiLinkRipEnabled(): boolean
  getMaxCollectionTracks(): number
  canRipLive(isAdmin: boolean): boolean
  canServeCache(isAdmin: boolean): boolean
  canRipAlbum(isAdmin: boolean): boolean
  canRipPlaylist(isAdmin: boolean): boolean
  canRipTxt(isAdmin: boolean): boolean
  canRipMultiLink(isAdmin: boolean): boolean
  setSetting<K extends keyof BotSettings>(
    key: K,
    value: BotSettings[K],
  ): Promise<BotSettings>
  cycleRippingMode(): Promise<RippingMode>
  toggleAlbumRip(): Promise<boolean>
  togglePlaylistRip(): Promise<boolean>
  toggleTxtRip(): Promise<boolean>
  toggleMultiLinkRip(): Promise<boolean>
  setMaxCollectionTracks(limit: number): Promise<number>
}

export class SettingsService implements ISettingsService {
  private readonly _db?: AppDatabase
  private _cachedSettings: BotSettings = { ...DEFAULT_SETTINGS }

  constructor(db?: AppDatabase) {
    this._db = db
  }

  private get db(): AppDatabase {
    return this._db ?? defaultDb
  }

  async init(): Promise<void> {
    try {
      const rows = await this.db.select().from(settings)
      const dbSettings: Partial<BotSettings> = {}

      for (const row of rows) {
        if (row.key === 'ripping_mode') {
          const val = row.value as string
          if (val === 'live' || val === 'cache_only' || val === 'paused') {
            dbSettings.rippingMode = val
          }
        } else if (row.key === 'album_rip_enabled') {
          dbSettings.albumRipEnabled = Boolean(row.value)
        } else if (row.key === 'playlist_rip_enabled') {
          dbSettings.playlistRipEnabled = Boolean(row.value)
        } else if (row.key === 'txt_rip_enabled') {
          dbSettings.txtRipEnabled = Boolean(row.value)
        } else if (row.key === 'multi_link_rip_enabled') {
          dbSettings.multiLinkRipEnabled = Boolean(row.value)
        } else if (row.key === 'max_collection_tracks') {
          const num = Number(row.value)
          if (!Number.isNaN(num) && num >= 0) {
            dbSettings.maxCollectionTracks = num
          }
        }
      }

      this._cachedSettings = {
        ...DEFAULT_SETTINGS,
        ...dbSettings,
      }
      info('Settings loaded into memory', { settings: this._cachedSettings })
    } catch (err) {
      error('Failed to load settings from database, using defaults', {
        error: String(err),
      })
      this._cachedSettings = { ...DEFAULT_SETTINGS }
    }
  }

  getSettings(): BotSettings {
    return { ...this._cachedSettings }
  }

  getRippingMode(): RippingMode {
    return this._cachedSettings.rippingMode
  }

  isAlbumRipEnabled(): boolean {
    return this._cachedSettings.albumRipEnabled
  }

  isPlaylistRipEnabled(): boolean {
    return this._cachedSettings.playlistRipEnabled
  }

  isTxtRipEnabled(): boolean {
    return this._cachedSettings.txtRipEnabled
  }

  isMultiLinkRipEnabled(): boolean {
    return this._cachedSettings.multiLinkRipEnabled
  }

  getMaxCollectionTracks(): number {
    return this._cachedSettings.maxCollectionTracks
  }

  canRipLive(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.rippingMode === 'live'
  }

  canServeCache(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.rippingMode !== 'paused'
  }

  canRipAlbum(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.albumRipEnabled
  }

  canRipPlaylist(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.playlistRipEnabled
  }

  canRipTxt(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.txtRipEnabled
  }

  canRipMultiLink(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.multiLinkRipEnabled
  }

  async setSetting<K extends keyof BotSettings>(
    key: K,
    value: BotSettings[K],
  ): Promise<BotSettings> {
    const dbKeyMap: Record<keyof BotSettings, string> = {
      rippingMode: 'ripping_mode',
      albumRipEnabled: 'album_rip_enabled',
      playlistRipEnabled: 'playlist_rip_enabled',
      txtRipEnabled: 'txt_rip_enabled',
      multiLinkRipEnabled: 'multi_link_rip_enabled',
      maxCollectionTracks: 'max_collection_tracks',
    }

    const dbKey = dbKeyMap[key]
    this._cachedSettings[key] = value

    try {
      await this.db
        .insert(settings)
        .values({
          key: dbKey,
          value: value as unknown as Record<string, unknown>,
          updatedAt: new Date(),
        })
        .onConflictDoUpdate({
          target: settings.key,
          set: {
            value: value as unknown as Record<string, unknown>,
            updatedAt: new Date(),
          },
        })
      debug('Setting persisted to database', { key: dbKey, value })
    } catch (err) {
      error('Failed to persist setting to database', {
        key: dbKey,
        value,
        error: String(err),
      })
    }

    return { ...this._cachedSettings }
  }

  async cycleRippingMode(): Promise<RippingMode> {
    const current = this._cachedSettings.rippingMode
    let next: RippingMode
    if (current === 'live') {
      next = 'cache_only'
    } else if (current === 'cache_only') {
      next = 'paused'
    } else {
      next = 'live'
    }

    await this.setSetting('rippingMode', next)
    return next
  }

  async toggleAlbumRip(): Promise<boolean> {
    const next = !this._cachedSettings.albumRipEnabled
    await this.setSetting('albumRipEnabled', next)
    return next
  }

  async togglePlaylistRip(): Promise<boolean> {
    const next = !this._cachedSettings.playlistRipEnabled
    await this.setSetting('playlistRipEnabled', next)
    return next
  }

  async toggleTxtRip(): Promise<boolean> {
    const next = !this._cachedSettings.txtRipEnabled
    await this.setSetting('txtRipEnabled', next)
    return next
  }

  async toggleMultiLinkRip(): Promise<boolean> {
    const next = !this._cachedSettings.multiLinkRipEnabled
    await this.setSetting('multiLinkRipEnabled', next)
    return next
  }

  async setMaxCollectionTracks(limit: number): Promise<number> {
    const validLimit = Math.max(0, limit)
    await this.setSetting('maxCollectionTracks', validLimit)
    return validLimit
  }
}

export const settingsService = new SettingsService()
