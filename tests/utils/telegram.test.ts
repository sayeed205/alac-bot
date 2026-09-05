import { describe, expect, it, mock } from 'bun:test'

import type { InputText, Message, TelegramClient } from '@mtcute/bun'

import {
  editMessageSafe,
  extractFloodWaitSeconds,
  sendTextSafe,
} from '@/utils/telegram.ts'

describe('Telegram Utilities', () => {
  describe('extractFloodWaitSeconds', () => {
    it('extracts seconds from standard FLOOD_WAIT_X', () => {
      expect(extractFloodWaitSeconds(new Error('FLOOD_WAIT_15'))).toBe(15)
      expect(
        extractFloodWaitSeconds(new Error('RPC_ERROR 420: FLOOD_WAIT_41184')),
      ).toBe(41184)
      expect(extractFloodWaitSeconds({ text: 'FLOOD_PREMIUM_WAIT_30' })).toBe(
        30,
      )
      expect(extractFloodWaitSeconds({ message: 'SLOWMODE_WAIT_5' })).toBe(5)
    })

    it('returns null for non-flood errors or empty values', () => {
      expect(extractFloodWaitSeconds(null)).toBeNull()
      expect(extractFloodWaitSeconds(undefined)).toBeNull()
      expect(
        extractFloodWaitSeconds(new Error('CHAT_WRITE_FORBIDDEN')),
      ).toBeNull()
    })
  })

  describe('editMessageSafe', () => {
    it('returns true on successful edit', async () => {
      const fakeTg = {
        editMessage: mock(() => Promise.resolve({} as Message)),
      } as unknown as TelegramClient

      const result = await editMessageSafe(fakeTg, {
        chatId: 123,
        message: 456,
        text: 'hello' as InputText,
      })

      expect(result).toBe(true)
      expect(fakeTg.editMessage).toHaveBeenCalledTimes(1)
    })

    it('treats MESSAGE_NOT_MODIFIED as success', async () => {
      const fakeTg = {
        editMessage: mock(() =>
          Promise.reject(new Error('400: MESSAGE_NOT_MODIFIED')),
        ),
      } as unknown as TelegramClient

      const result = await editMessageSafe(fakeTg, {
        chatId: 123,
        message: 456,
        text: 'hello' as InputText,
      })

      expect(result).toBe(true)
    })

    it('returns false immediately when block=false on FLOOD_WAIT without sleeping', async () => {
      const start = Date.now()
      const fakeTg = {
        editMessage: mock(() => Promise.reject(new Error('FLOOD_WAIT_60'))),
      } as unknown as TelegramClient

      const result = await editMessageSafe(fakeTg, {
        chatId: 123,
        message: 456,
        text: 'progress' as InputText,
        block: false,
      })

      const elapsed = Date.now() - start
      expect(result).toBe(false)
      expect(elapsed).toBeLessThan(100) // Immediate non-blocking return
      expect(fakeTg.editMessage).toHaveBeenCalledTimes(1)
    })

    it('returns false when flood wait exceeds maxWaitSec even when block=true', async () => {
      const start = Date.now()
      const fakeTg = {
        editMessage: mock(() => Promise.reject(new Error('FLOOD_WAIT_41184'))),
      } as unknown as TelegramClient

      const result = await editMessageSafe(fakeTg, {
        chatId: 123,
        message: 456,
        text: 'summary' as InputText,
        block: true,
        maxWaitSec: 5,
      })

      const elapsed = Date.now() - start
      expect(result).toBe(false)
      expect(elapsed).toBeLessThan(100)
    })
  })

  describe('sendTextSafe', () => {
    it('returns true on successful send', async () => {
      const fakeTg = {
        sendText: mock(() => Promise.resolve({} as Message)),
      } as unknown as TelegramClient

      const result = await sendTextSafe(fakeTg, {
        chatId: 123,
        text: 'test' as InputText,
      })

      expect(result).toBe(true)
      expect(fakeTg.sendText).toHaveBeenCalledTimes(1)
    })

    it('returns false immediately when block=false on FLOOD_WAIT', async () => {
      const fakeTg = {
        sendText: mock(() => Promise.reject(new Error('FLOOD_WAIT_100'))),
      } as unknown as TelegramClient

      const result = await sendTextSafe(fakeTg, {
        chatId: 123,
        text: 'test' as InputText,
        block: false,
      })

      expect(result).toBe(false)
    })
  })
})
