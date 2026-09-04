import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { writeFileSync } from 'node:fs'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher, type MessageContext } from '@mtcute/dispatcher'

import { registerSpecCommand } from '@/modules/alac/commands/spec.ts'
import type { CommandContext } from '@/modules/alac/commands/types.ts'
import type { IRipQueue } from '@/modules/alac/queue.ts'
import type { ITrackRipper } from '@/modules/alac/ripper.ts'
import type { IAlacService } from '@/modules/alac/service.ts'
import type { IAuthService } from '@/modules/auth/service.ts'

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

describe('ALAC Spectrogram Command (/spec & /spectogram)', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let ctx: CommandContext

  beforeEach(() => {
    fakeTg = {
      sendMedia: mock(() => Promise.resolve({ id: 200 })),
      sendText: mock(() => Promise.resolve({ id: 101 })),
      editMessage: mock(() => Promise.resolve({ id: 101 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      downloadToFile: mock((path: string) => {
        writeFileSync(path, 'RIFFdummyWAVEfmt ')
        return Promise.resolve()
      }),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    mockAuth = {
      isAdmin: mock((id: number) => id === 1),
      isAuthorized: mock((id: number) => Promise.resolve(id === 1 || id === 2)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    dp = Dispatcher.for(fakeTg)
    ctx = {
      dp,
      tg: fakeTg,
      service: {} as unknown as IAlacService,
      ripper: {} as unknown as ITrackRipper,
      queue: {} as unknown as IRipQueue,
      auth: mockAuth,
    }

    registerSpecCommand(ctx)
  })

  async function triggerMessage(
    text: string,
    senderId: number,
    replyMedia?: unknown,
  ) {
    let capturedReply = ''
    const fakeMsg = {
      text,
      sender: { id: senderId, type: 'user' },
      chat: { id: senderId, type: 'user' },
      id: 42,
      replyText: mock((t: { text: string } | string) => {
        capturedReply = typeof t === 'string' ? t : t.text
        return Promise.resolve()
      }),
      getReplyTo: mock(() =>
        Promise.resolve(
          replyMedia
            ? {
                id: 99,
                media: replyMedia,
              }
            : null,
        ),
      ),
      getCapturedReply: () => capturedReply,
    } as unknown as MessageContext & { getCapturedReply: () => string }

    const internal = dp as unknown as DispatcherInternal
    const handlers = internal._groups.get(0)?.get('new_message') || []
    for (const h of handlers) {
      if (await h.check(fakeMsg)) {
        await h.callback(fakeMsg)
      }
    }
    return fakeMsg
  }

  it('blocks unauthorized users from using /spec', async () => {
    const msg = await triggerMessage('/spec', 999)
    expect(msg.replyText).not.toHaveBeenCalled()
    expect(fakeTg.sendText).not.toHaveBeenCalled()
  })

  it('shows usage guide when user does not reply to any media', async () => {
    const msg = await triggerMessage('/spec', 1)
    expect(msg.replyText).toHaveBeenCalled()
    expect(msg.getCapturedReply()).toContain('Audio Spectrogram Analyzer')
  })

  it('warns user when replied message is not audio', async () => {
    const msg = await triggerMessage('/spec', 1, { type: 'photo' })
    expect(msg.replyText).toHaveBeenCalled()
    expect(msg.getCapturedReply()).toContain('Unsupported Media')
  })

  it('handles /spectogram alias and document audio formats', async () => {
    await triggerMessage('/spectogram', 1, {
      type: 'document',
      mimeType: 'audio/flac',
      fileName: 'test_track.flac',
    })

    expect(fakeTg.sendText).toHaveBeenCalled()
    expect(fakeTg.downloadToFile).toHaveBeenCalled()
  })

  it('handles /spek and /spectrogram aliases', async () => {
    const msg1 = await triggerMessage('/spek', 1)
    expect(msg1.replyText).toHaveBeenCalled()

    const msg2 = await triggerMessage('/spectrogram', 1)
    expect(msg2.replyText).toHaveBeenCalled()
  })
})
