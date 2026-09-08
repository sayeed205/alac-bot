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
  isArtistRipEnabled(): boolean
  isTxtRipEnabled(): boolean
  isMultiLinkRipEnabled(): boolean
  getMaxCollectionTracks(): number
  isAutoDumpEnabled(): boolean
  getAutoDumpStorefronts(): string[]
  canRipLive(isAdmin: boolean): boolean
  canServeCache(isAdmin: boolean): boolean
  canRipAlbum(isAdmin: boolean): boolean
  canRipPlaylist(isAdmin: boolean): boolean
  canRipArtist(isAdmin: boolean): boolean
  canRipTxt(isAdmin: boolean): boolean
  canRipMultiLink(isAdmin: boolean): boolean
  setSetting<K extends keyof BotSettings>(
    key: K,
    value: BotSettings[K],
  ): Promise<BotSettings>
  cycleRippingMode(): Promise<RippingMode>
  toggleAlbumRip(): Promise<boolean>
  togglePlaylistRip(): Promise<boolean>
  toggleArtistRip(): Promise<boolean>
  toggleTxtRip(): Promise<boolean>
  toggleMultiLinkRip(): Promise<boolean>
  setMaxCollectionTracks(limit: number): Promise<number>
  toggleAutoDump(): Promise<boolean>
  addAutoDumpStorefront(sf: string): Promise<string[]>
  removeAutoDumpStorefront(sf: string): Promise<string[]>
  setAutoDumpStorefronts(sfs: string[]): Promise<string[]>
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
        } else if (row.key === 'artist_rip_enabled') {
          dbSettings.artistRipEnabled = Boolean(row.value)
        } else if (row.key === 'txt_rip_enabled') {
          dbSettings.txtRipEnabled = Boolean(row.value)
        } else if (row.key === 'multi_link_rip_enabled') {
          dbSettings.multiLinkRipEnabled = Boolean(row.value)
        } else if (row.key === 'max_collection_tracks') {
          const num = Number(row.value)
          if (!Number.isNaN(num) && num >= 0) {
            dbSettings.maxCollectionTracks = num
          }
        } else if (row.key === 'auto_dump_enabled') {
          dbSettings.autoDumpEnabled = Boolean(row.value)
        } else if (row.key === 'auto_dump_storefronts') {
          if (Array.isArray(row.value)) {
            const list = (row.value as unknown[])
              .map((s) => String(s).toLowerCase().trim())
              .filter(Boolean)
            if (list.length > 0) {
              dbSettings.autoDumpStorefronts = list
            }
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

  isArtistRipEnabled(): boolean {
    return this._cachedSettings.artistRipEnabled
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

  isAutoDumpEnabled(): boolean {
    return this._cachedSettings.autoDumpEnabled
  }

  getAutoDumpStorefronts(): string[] {
    return [...this._cachedSettings.autoDumpStorefronts]
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

  canRipArtist(isAdmin: boolean): boolean {
    if (isAdmin) return true
    return this._cachedSettings.artistRipEnabled
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
      artistRipEnabled: 'artist_rip_enabled',
      txtRipEnabled: 'txt_rip_enabled',
      multiLinkRipEnabled: 'multi_link_rip_enabled',
      maxCollectionTracks: 'max_collection_tracks',
      autoDumpEnabled: 'auto_dump_enabled',
      autoDumpStorefronts: 'auto_dump_storefronts',
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

  async toggleArtistRip(): Promise<boolean> {
    const next = !this._cachedSettings.artistRipEnabled
    await this.setSetting('artistRipEnabled', next)
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

  async toggleAutoDump(): Promise<boolean> {
    const next = !this._cachedSettings.autoDumpEnabled
    await this.setSetting('autoDumpEnabled', next)
    return next
  }

  async addAutoDumpStorefront(sf: string): Promise<string[]> {
    const clean = sf.toLowerCase().trim()
    if (!clean) return this.getAutoDumpStorefronts()
    const set = new Set(this._cachedSettings.autoDumpStorefronts)
    set.add(clean)
    const next = Array.from(set)
    await this.setSetting('autoDumpStorefronts', next)
    return next
  }

  async removeAutoDumpStorefront(sf: string): Promise<string[]> {
    const clean = sf.toLowerCase().trim()
    const filtered = this._cachedSettings.autoDumpStorefronts.filter(
      (s) => s !== clean,
    )
    const next = filtered.length > 0 ? filtered : ['us']
    await this.setSetting('autoDumpStorefronts', next)
    return next
  }

  async setAutoDumpStorefronts(sfs: string[]): Promise<string[]> {
    const cleaned = Array.from(
      new Set(sfs.map((s) => s.toLowerCase().trim()).filter(Boolean)),
    )
    const next = cleaned.length > 0 ? cleaned : ['us']
    await this.setSetting('autoDumpStorefronts', next)
    return next
  }
}

export const settingsService = new SettingsService()
