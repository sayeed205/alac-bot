import { beforeEach, describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher, type MessageContext } from '@mtcute/dispatcher'

import type { IAuthService } from '@/modules/auth/service.ts'
import {
  buildSettingsKeyboard,
  registerSettingsCommands,
  renderSettingsText,
} from '@/modules/settings/commands/settings.ts'
import type { ISettingsService } from '@/modules/settings/service.ts'
import type { BotSettings } from '@/modules/settings/types.ts'

interface DispatcherInternal {
  _groups: Map<
    number,
    Map<
      string,
      Array<{
        check: (ctx: unknown) => Promise<boolean>
        callback: (ctx: unknown) => Promise<void>
      }>
    >
  >
}

describe('Settings Commands & Callbacks', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockSettings: BotSettings
  let mockSettingsService: ISettingsService

  const ADMIN_ID = 1000
  const REGULAR_USER_ID = 2000

  beforeEach(() => {
    process.env.ADMIN_ID = String(ADMIN_ID)

    mockSettings = {
      rippingMode: 'live',
      albumRipEnabled: true,
      playlistRipEnabled: true,
      artistRipEnabled: true,
      txtRipEnabled: true,
      multiLinkRipEnabled: true,
      maxCollectionTracks: 50,
      autoDumpEnabled: true,
      autoDumpStorefronts: ['us'],
    }

    mockSettingsService = {
      init: mock(() => Promise.resolve()),
      getSettings: mock(() => ({ ...mockSettings })),
      getRippingMode: mock(() => mockSettings.rippingMode),
      isAlbumRipEnabled: mock(() => mockSettings.albumRipEnabled),
      isPlaylistRipEnabled: mock(() => mockSettings.playlistRipEnabled),
      isArtistRipEnabled: mock(() => mockSettings.artistRipEnabled),
      isTxtRipEnabled: mock(() => mockSettings.txtRipEnabled),
      isMultiLinkRipEnabled: mock(() => mockSettings.multiLinkRipEnabled),
      getMaxCollectionTracks: mock(() => mockSettings.maxCollectionTracks),
      isAutoDumpEnabled: mock(() => mockSettings.autoDumpEnabled),
      getAutoDumpStorefronts: mock(() => [...mockSettings.autoDumpStorefronts]),
      canRipLive: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.rippingMode === 'live',
      ),
      canServeCache: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.rippingMode !== 'paused',
      ),
      canRipAlbum: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.albumRipEnabled,
      ),
      canRipPlaylist: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.playlistRipEnabled,
      ),
      canRipArtist: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.artistRipEnabled,
      ),
      canRipTxt: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.txtRipEnabled,
      ),
      canRipMultiLink: mock(
        (isAdmin: boolean) => isAdmin || mockSettings.multiLinkRipEnabled,
      ),
      setSetting: mock(async (key, val) => {
        ;(mockSettings as unknown as Record<string, unknown>)[key] = val
        return { ...mockSettings }
      }),
      cycleRippingMode: mock(async () => {
        if (mockSettings.rippingMode === 'live')
          mockSettings.rippingMode = 'cache_only'
        else if (mockSettings.rippingMode === 'cache_only')
          mockSettings.rippingMode = 'paused'
        else mockSettings.rippingMode = 'live'
        return mockSettings.rippingMode
      }),
      toggleAlbumRip: mock(async () => {
        mockSettings.albumRipEnabled = !mockSettings.albumRipEnabled
        return mockSettings.albumRipEnabled
      }),
      togglePlaylistRip: mock(async () => {
        mockSettings.playlistRipEnabled = !mockSettings.playlistRipEnabled
        return mockSettings.playlistRipEnabled
      }),
      toggleArtistRip: mock(async () => {
        mockSettings.artistRipEnabled = !mockSettings.artistRipEnabled
        return mockSettings.artistRipEnabled
      }),
      toggleTxtRip: mock(async () => {
        mockSettings.txtRipEnabled = !mockSettings.txtRipEnabled
        return mockSettings.txtRipEnabled
      }),
      toggleMultiLinkRip: mock(async () => {
        mockSettings.multiLinkRipEnabled = !mockSettings.multiLinkRipEnabled
        return mockSettings.multiLinkRipEnabled
      }),
      setMaxCollectionTracks: mock(async (num: number) => {
        mockSettings.maxCollectionTracks = Math.max(0, num)
        return mockSettings.maxCollectionTracks
      }),
      toggleAutoDump: mock(async () => {
        mockSettings.autoDumpEnabled = !mockSettings.autoDumpEnabled
        return mockSettings.autoDumpEnabled
      }),
      addAutoDumpStorefront: mock(async (sf: string) => {
        mockSettings.autoDumpStorefronts.push(sf.toLowerCase())
        return mockSettings.autoDumpStorefronts
      }),
      removeAutoDumpStorefront: mock(async (sf: string) => {
        mockSettings.autoDumpStorefronts =
          mockSettings.autoDumpStorefronts.filter((s) => s !== sf.toLowerCase())
        return mockSettings.autoDumpStorefronts
      }),
      setAutoDumpStorefronts: mock(async (sfs: string[]) => {
        mockSettings.autoDumpStorefronts = [...sfs]
        return mockSettings.autoDumpStorefronts
      }),
    }

    fakeTg = {
      deleteMessagesById: mock(() => Promise.resolve()),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      sendText: mock(() => Promise.resolve({ id: 1 })),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    const mockAuth: IAuthService = {
      isAdmin: mock((id: number) => id === ADMIN_ID),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    dp = Dispatcher.for(fakeTg)
    registerSettingsCommands(dp, fakeTg, mockSettingsService, mockAuth)
  })

  async function triggerMessage(text: string, senderId: number) {
    const fakeMsg = {
      text,
      sender: { id: senderId, type: 'user' },
      chat: { id: senderId, type: 'user' },
      id: 42,
      answerText: mock(() => Promise.resolve()),
      replyText: mock(() => Promise.resolve()),
    } as unknown as MessageContext

    const internal = dp as unknown as DispatcherInternal
    const handlers = internal._groups.get(0)?.get('new_message') || []
    for (const h of handlers) {
      if (await h.check(fakeMsg)) {
        await h.callback(fakeMsg)
      }
    }
    return fakeMsg
  }

  async function triggerCallbackQuery(data: string, userId: number) {
    const answeredTexts: string[] = []
    const fakeQuery = {
      _name: 'callback_query',
      dataStr: data,
      raw: { data: Buffer.from(data) },
      user: { id: userId, displayName: 'Test User' },
      chat: { id: 12345 },
      messageId: 100,
      answer: mock((options?: { text?: string; alert?: boolean }) => {
        if (options?.text) answeredTexts.push(options.text)
        return Promise.resolve()
      }),
    }

    const internal = dp as unknown as DispatcherInternal
    const handlers = internal._groups.get(0)?.get('callback_query') || []
    for (const h of handlers) {
      if (await h.check(fakeQuery)) {
        await h.callback(fakeQuery)
      }
    }
    return { fakeQuery, answeredTexts }
  }

  describe('Keyboard and Text Renderers', () => {
    it('renders correct text description according to mode', () => {
      const textLive = renderSettingsText(mockSettings)
      expect(textLive).toContain('Live Ripping')
      expect(textLive).toContain('.TXT File Ripping')
      expect(textLive).toContain('Multi-Link Ripping')
      expect(textLive).toContain('50 tracks')

      mockSettings.rippingMode = 'cache_only'
      const textCache = renderSettingsText(mockSettings)
      expect(textCache).toContain('Cache Only')

      mockSettings.rippingMode = 'paused'
      const textPaused = renderSettingsText(mockSettings)
      expect(textPaused).toContain('Paused')
    })

    it('builds keyboard buttons matching settings state', () => {
      const kb = buildSettingsKeyboard(mockSettings)
      expect(kb).toBeDefined()
    })
  })

  describe('/settings Command', () => {
    it('ignores /settings from non-admin', async () => {
      await triggerMessage('/settings', REGULAR_USER_ID)
      expect(fakeTg.sendText).not.toHaveBeenCalled()
    })

    it('renders settings dashboard for admin', async () => {
      await triggerMessage('/settings', ADMIN_ID)
      expect(fakeTg.sendText).toHaveBeenCalled()
    })

    it('handles /settings mode <val> subcommand', async () => {
      const msg = await triggerMessage('/settings mode cache_only', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'rippingMode',
        'cache_only',
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings album <val> subcommand', async () => {
      const msg = await triggerMessage('/settings album off', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'albumRipEnabled',
        false,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings artist <val> subcommand', async () => {
      const msg = await triggerMessage('/settings artist off', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'artistRipEnabled',
        false,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings txt <val> subcommand', async () => {
      const msg = await triggerMessage('/settings txt off', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'txtRipEnabled',
        false,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings multilink <val> subcommand', async () => {
      const msg = await triggerMessage('/settings multilink off', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'multiLinkRipEnabled',
        false,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings autodump <val> subcommand', async () => {
      const msg = await triggerMessage('/settings autodump off', ADMIN_ID)
      expect(mockSettingsService.setSetting).toHaveBeenCalledWith(
        'autoDumpEnabled',
        false,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings storefronts add subcommand', async () => {
      const msg = await triggerMessage('/settings storefronts add jp', ADMIN_ID)
      expect(mockSettingsService.addAutoDumpStorefront).toHaveBeenCalledWith(
        'jp',
      )
      expect(msg.answerText).toHaveBeenCalled()
    })

    it('handles /settings limit <val> subcommand', async () => {
      const msg = await triggerMessage('/settings limit 100', ADMIN_ID)
      expect(mockSettingsService.setMaxCollectionTracks).toHaveBeenCalledWith(
        100,
      )
      expect(msg.answerText).toHaveBeenCalled()
    })
  })

  describe('Callback Queries', () => {
    it('rejects callback queries from non-admin', async () => {
      const { fakeQuery, answeredTexts } = await triggerCallbackQuery(
        'settings:mode',
        REGULAR_USER_ID,
      )
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(answeredTexts).toContain('Unauthorized. Owner only.')
      expect(mockSettingsService.cycleRippingMode).not.toHaveBeenCalled()
    })

    it('cycles mode on settings:mode callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:mode',
        ADMIN_ID,
      )
      expect(mockSettingsService.cycleRippingMode).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles album on settings:album callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:album',
        ADMIN_ID,
      )
      expect(mockSettingsService.toggleAlbumRip).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles playlist on settings:playlist callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:playlist',
        ADMIN_ID,
      )
      expect(mockSettingsService.togglePlaylistRip).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles artist on settings:artist callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:artist',
        ADMIN_ID,
      )
      expect(mockSettingsService.toggleArtistRip).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles txt on settings:txt callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery('settings:txt', ADMIN_ID)
      expect(mockSettingsService.toggleTxtRip).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles multilink on settings:multilink callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:multilink',
        ADMIN_ID,
      )
      expect(mockSettingsService.toggleMultiLinkRip).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('toggles autodump on settings:autodump callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:autodump',
        ADMIN_ID,
      )
      expect(mockSettingsService.toggleAutoDump).toHaveBeenCalled()
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('sets limit on settings:limit:<num> callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:limit:25',
        ADMIN_ID,
      )
      expect(mockSettingsService.setMaxCollectionTracks).toHaveBeenCalledWith(
        25,
      )
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.editMessage).toHaveBeenCalled()
    })

    it('deletes message on settings:close callback', async () => {
      const { fakeQuery } = await triggerCallbackQuery(
        'settings:close',
        ADMIN_ID,
      )
      expect(fakeQuery.answer).toHaveBeenCalled()
      expect(fakeTg.deleteMessagesById).toHaveBeenCalled()
    })
  })
})
