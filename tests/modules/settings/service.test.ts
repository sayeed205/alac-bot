import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'bun:test'

import type { AppDatabase } from '@/db/index.ts'
import { SettingsService } from '@/modules/settings/service.ts'

import { setupTestDb } from '../../test-db.ts'

describe('SettingsService', () => {
  let db: AppDatabase
  let cleanDb: () => Promise<void>
  let close: () => Promise<void>
  let service: SettingsService

  beforeAll(async () => {
    const testEnv = await setupTestDb()
    db = testEnv.db
    cleanDb = testEnv.cleanDb
    close = testEnv.close
    service = new SettingsService(db)
  })

  beforeEach(async () => {
    await cleanDb()
    await service.init()
  })

  afterAll(async () => {
    await close()
  })

  it('initializes with default settings', () => {
    const settings = service.getSettings()
    expect(settings.rippingMode).toBe('live')
    expect(settings.albumRipEnabled).toBe(true)
    expect(settings.playlistRipEnabled).toBe(true)
    expect(settings.artistRipEnabled).toBe(true)
    expect(settings.txtRipEnabled).toBe(true)
    expect(settings.multiLinkRipEnabled).toBe(true)
    expect(settings.maxCollectionTracks).toBe(50)
  })

  it('cycles ripping mode correctly', async () => {
    expect(service.getRippingMode()).toBe('live')

    const mode1 = await service.cycleRippingMode()
    expect(mode1).toBe('cache_only')
    expect(service.getRippingMode()).toBe('cache_only')

    const mode2 = await service.cycleRippingMode()
    expect(mode2).toBe('paused')
    expect(service.getRippingMode()).toBe('paused')

    const mode3 = await service.cycleRippingMode()
    expect(mode3).toBe('live')
    expect(service.getRippingMode()).toBe('live')
  })

  it('toggles album, playlist, and artist settings', async () => {
    expect(service.isAlbumRipEnabled()).toBe(true)
    const albumRes = await service.toggleAlbumRip()
    expect(albumRes).toBe(false)
    expect(service.isAlbumRipEnabled()).toBe(false)

    expect(service.isPlaylistRipEnabled()).toBe(true)
    const playlistRes = await service.togglePlaylistRip()
    expect(playlistRes).toBe(false)
    expect(service.isPlaylistRipEnabled()).toBe(false)

    expect(service.isArtistRipEnabled()).toBe(true)
    const artistRes = await service.toggleArtistRip()
    expect(artistRes).toBe(false)
    expect(service.isArtistRipEnabled()).toBe(false)
  })

  it('toggles txt and multi-link settings', async () => {
    expect(service.isTxtRipEnabled()).toBe(true)
    const txtRes = await service.toggleTxtRip()
    expect(txtRes).toBe(false)
    expect(service.isTxtRipEnabled()).toBe(false)

    expect(service.isMultiLinkRipEnabled()).toBe(true)
    const multiRes = await service.toggleMultiLinkRip()
    expect(multiRes).toBe(false)
    expect(service.isMultiLinkRipEnabled()).toBe(false)
  })

  it('updates max collection tracks and clamps negative numbers', async () => {
    await service.setMaxCollectionTracks(100)
    expect(service.getMaxCollectionTracks()).toBe(100)

    await service.setMaxCollectionTracks(-10)
    expect(service.getMaxCollectionTracks()).toBe(0)
  })

  it('persists settings to database and restores them on init', async () => {
    await service.setSetting('rippingMode', 'cache_only')
    await service.setSetting('albumRipEnabled', false)
    await service.setSetting('artistRipEnabled', false)
    await service.setSetting('txtRipEnabled', false)
    await service.setSetting('multiLinkRipEnabled', false)
    await service.setSetting('maxCollectionTracks', 25)

    const freshService = new SettingsService(db)
    await freshService.init()

    const settings = freshService.getSettings()
    expect(settings.rippingMode).toBe('cache_only')
    expect(settings.albumRipEnabled).toBe(false)
    expect(settings.playlistRipEnabled).toBe(true)
    expect(settings.artistRipEnabled).toBe(false)
    expect(settings.txtRipEnabled).toBe(false)
    expect(settings.multiLinkRipEnabled).toBe(false)
    expect(settings.maxCollectionTracks).toBe(25)
  })

  describe('Permission & Bypass Checks', () => {
    it('admin always bypasses ripping restrictions', async () => {
      await service.setSetting('rippingMode', 'paused')
      await service.setSetting('albumRipEnabled', false)
      await service.setSetting('playlistRipEnabled', false)
      await service.setSetting('artistRipEnabled', false)
      await service.setSetting('txtRipEnabled', false)
      await service.setSetting('multiLinkRipEnabled', false)

      expect(service.canRipLive(true)).toBe(true)
      expect(service.canServeCache(true)).toBe(true)
      expect(service.canRipAlbum(true)).toBe(true)
      expect(service.canRipPlaylist(true)).toBe(true)
      expect(service.canRipArtist(true)).toBe(true)
      expect(service.canRipTxt(true)).toBe(true)
      expect(service.canRipMultiLink(true)).toBe(true)
    })

    it('enforces live ripping restrictions for non-admin', async () => {
      await service.setSetting('rippingMode', 'live')
      expect(service.canRipLive(false)).toBe(true)
      expect(service.canServeCache(false)).toBe(true)

      await service.setSetting('rippingMode', 'cache_only')
      expect(service.canRipLive(false)).toBe(false)
      expect(service.canServeCache(false)).toBe(true)

      await service.setSetting('rippingMode', 'paused')
      expect(service.canRipLive(false)).toBe(false)
      expect(service.canServeCache(false)).toBe(false)
    })

    it('enforces collection restrictions for non-admin', async () => {
      await service.setSetting('albumRipEnabled', false)
      expect(service.canRipAlbum(false)).toBe(false)

      await service.setSetting('albumRipEnabled', true)
      expect(service.canRipAlbum(false)).toBe(true)

      await service.setSetting('playlistRipEnabled', false)
      expect(service.canRipPlaylist(false)).toBe(false)

      await service.setSetting('playlistRipEnabled', true)
      expect(service.canRipPlaylist(false)).toBe(true)

      await service.setSetting('artistRipEnabled', false)
      expect(service.canRipArtist(false)).toBe(false)

      await service.setSetting('artistRipEnabled', true)
      expect(service.canRipArtist(false)).toBe(true)
    })

    it('enforces txt and multi-link restrictions for non-admin', async () => {
      await service.setSetting('txtRipEnabled', false)
      expect(service.canRipTxt(false)).toBe(false)

      await service.setSetting('txtRipEnabled', true)
      expect(service.canRipTxt(false)).toBe(true)

      await service.setSetting('multiLinkRipEnabled', false)
      expect(service.canRipMultiLink(false)).toBe(false)

      await service.setSetting('multiLinkRipEnabled', true)
      expect(service.canRipMultiLink(false)).toBe(true)
    })
  })
})
