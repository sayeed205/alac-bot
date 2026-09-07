import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'
import { Dispatcher } from '@mtcute/dispatcher'

import { type ActiveRipJob, activeJobs } from '@/modules/alac/commands/rip.ts'
import {
  lastRefreshTimeByMsg,
  lastStatusMsgByChat,
  registerStatusCommand,
  renderStatusDashboard,
} from '@/modules/alac/commands/status.ts'
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

describe('Status Command & Dashboard', () => {
  let fakeTg: TelegramClient
  let dp: Dispatcher
  let mockAuth: IAuthService
  let mockService: IAlacService
  let mockQueue: IRipQueue
  let mockRipper: ITrackRipper
  let ctx: CommandContext

  beforeEach(() => {
    activeJobs.clear()
    lastStatusMsgByChat.clear()
    lastRefreshTimeByMsg.clear()

    fakeTg = {
      deleteMessagesById: mock(() => Promise.resolve()),
      editMessage: mock(() => Promise.resolve({ id: 1 })),
      sendText: mock(() => Promise.resolve({ id: 101 })),
      onUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onRawUpdate: { add: mock(() => {}), remove: mock(() => {}) },
      onError: { add: mock(() => {}), remove: mock(() => {}) },
    } as unknown as TelegramClient

    dp = Dispatcher.for(fakeTg)

    mockAuth = {
      isAdmin: mock((id: number) => id === 1),
      isAuthorized: mock(() => Promise.resolve(true)),
      authorize: mock(() => Promise.resolve({ newlyAdded: true })),
      revoke: mock(() => Promise.resolve({ revoked: true })),
      listAuthorized: mock(() => Promise.resolve([])),
    }

    mockService = {} as unknown as IAlacService
    mockQueue = {
      enqueue: mock((task) =>
        task(new AbortController().signal),
      ) as unknown as IRipQueue['enqueue'],
      getPendingCount: mock(() => 0),
      isProcessing: mock(() => false),
      clear: mock(() => {}),
    }
    mockRipper = {} as unknown as ITrackRipper

    ctx = {
      dp,
      tg: fakeTg,
      service: mockService,
      queue: mockQueue,
      ripper: mockRipper,
      auth: mockAuth,
    }
  })

  afterEach(() => {
    activeJobs.clear()
    lastStatusMsgByChat.clear()
    lastRefreshTimeByMsg.clear()
  })

  async function dispatchMessage(text: string, userId = 1, chatId = 100) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('new_message') ?? []

    const repliedTexts: string[] = []
    const repliedMarkups: unknown[] = []
    const msg = {
      _name: 'new_message',
      text,
      sender: { id: userId, displayName: 'Test User' },
      chat: { id: chatId },
      getReplyTo: mock(() => Promise.resolve(null)),
      replyText: mock(
        (t: { text: string } | string, params?: { replyMarkup?: unknown }) => {
          const content = typeof t === 'string' ? t : t.text
          repliedTexts.push(content)
          repliedMarkups.push(params?.replyMarkup)
          return Promise.resolve({ id: 200, text: content })
        },
      ),
    }

    for (const h of handlers) {
      if (await h.check(msg)) {
        await h.callback(msg)
      }
    }

    return { repliedTexts, repliedMarkups }
  }

  async function dispatchCallback(
    data: string,
    userId = 1,
    chatId = 100,
    msgId = 200,
  ) {
    const internalDp = dp as unknown as DispatcherInternal
    const group0 = internalDp._groups.get(0)
    const handlers = group0?.get('callback_query') ?? []

    const answers: Array<{ text?: string; showAlert?: boolean }> = []
    const cq = {
      _name: 'callback_query',
      data,
      dataStr: data,
      raw: { data: Buffer.from(data) },
      sender: { id: userId, displayName: 'Test User' },
      user: { id: userId, displayName: 'Test User' },
      chat: { id: chatId },
      messageId: msgId,
      message: { id: msgId, chat: { id: chatId } },
      match: data.match(/^status:(refresh|page|cancel)(?::(.*))?$/),
      answer: mock((opts?: { text?: string; showAlert?: boolean }) => {
        answers.push(opts ?? {})
        return Promise.resolve()
      }),
    }

    for (const h of handlers) {
      if (await h.check(cq)) {
        await h.callback(cq)
      }
    }

    return { answers }
  }

  it('renders idle dashboard when activeJobs is empty', async () => {
    registerStatusCommand(ctx)
    const { repliedTexts } = await dispatchMessage('/status')

    expect(repliedTexts.length).toBe(1)
    expect(repliedTexts[0]).toContain('Rip Queue is Idle')
    expect(repliedTexts[0]).toContain('No active downloads')
  })

  it('renders active and queued jobs with rich MLTB details', async () => {
    const job1: ActiveRipJob = {
      id: 'job_1',
      chatId: 100,
      userId: 1,
      userName: 'Alice',
      jobHeader: 'Album: HIT ME HARD AND SOFT by Billie Eilish',
      totalTracks: 10,
      statusMsgId: 50,
      controller: new AbortController(),
      isCancelled: false,
      cachedCount: 2,
      rippedCount: 3,
      failedCount: 0,
      completed: false,
      queuePosition: 0,
      startTime: Date.now() - 45000,
      activeActionText: '📥 <b>Downloading:</b> CHIHIRO [12.4/45.2 MB]',
    }
    const job2: ActiveRipJob = {
      id: 'job_2',
      chatId: 100,
      userId: 2,
      userName: 'Bob',
      jobHeader: 'Album: GUTS by Olivia Rodrigo',
      totalTracks: 12,
      statusMsgId: 51,
      controller: new AbortController(),
      isCancelled: false,
      cachedCount: 0,
      rippedCount: 0,
      failedCount: 0,
      completed: false,
      queuePosition: 1,
      startTime: Date.now() - 5000,
    }

    activeJobs.set('job_1', job1)
    activeJobs.set('job_2', job2)

    registerStatusCommand(ctx)
    const { repliedTexts } = await dispatchMessage('/status')

    expect(repliedTexts.length).toBe(1)
    const card = repliedTexts[0]

    expect(card).toContain('Rip Queue & System Status')
    expect(card).toContain('Active Tasks:')
    expect(card).toContain('Queued:')
    // Active job details
    expect(card).toContain('HIT ME HARD AND SOFT')
    expect(card).toContain('CHIHIRO')
    expect(card).toContain('Downloading:')
    expect(card).toContain('50%')
    expect(card).toContain('2 cached')
    expect(card).toContain('3 ripped')
    expect(card).toContain('Alice')
    // Queued job details
    expect(card).toContain('GUTS')
    expect(card).toContain('In Queue (Position #1)')
    expect(card).toContain('Bob')
  })

  it('deletes previous status message in the chat when /status is run again', async () => {
    registerStatusCommand(ctx)

    await dispatchMessage('/status', 1, 100)
    expect(lastStatusMsgByChat.get(100)).toBe(200)

    // Run /status again in same chat
    await dispatchMessage('/status', 1, 100)
    expect(fakeTg.deleteMessagesById).toHaveBeenCalledWith(100, [200])
  })

  it('handles refresh button callback and applies 2.5s debounce', async () => {
    registerStatusCommand(ctx)

    // First refresh
    const res1 = await dispatchCallback('status:refresh:1', 1, 100, 200)
    expect(res1.answers[0].text).toBe('✅ Refreshed!')
    expect(fakeTg.editMessage).toHaveBeenCalled()

    // Immediate second refresh within 2.5s
    const res2 = await dispatchCallback('status:refresh:1', 1, 100, 200)
    expect(res2.answers[0].text).toContain('already up to date')
  })

  it('handles pagination across multiple queued jobs', async () => {
    for (let i = 1; i <= 5; i++) {
      activeJobs.set(`job_${i}`, {
        id: `job_${i}`,
        chatId: 100,
        userId: i,
        userName: `User_${i}`,
        jobHeader: `Job Number ${i}`,
        totalTracks: 5,
        statusMsgId: 100 + i,
        controller: new AbortController(),
        isCancelled: false,
        cachedCount: 0,
        rippedCount: 0,
        failedCount: 0,
        completed: false,
        queuePosition: i - 1,
        startTime: Date.now(),
      })
    }

    const { text: page1Text } = renderStatusDashboard(1)
    expect(page1Text.text).toContain('Job Number 1')
    expect(page1Text.text).toContain('Job Number 3')
    expect(page1Text.text).not.toContain('Job Number 4')

    const { text: page2Text } = renderStatusDashboard(2)
    expect(page2Text.text).toContain('Job Number 4')
    expect(page2Text.text).toContain('Job Number 5')
    expect(page2Text.text).not.toContain('Job Number 1')
  })

  it('allows job requester or admin to cancel job via status button', async () => {
    const jobController = new AbortController()
    const job: ActiveRipJob = {
      id: 'cancel_me',
      chatId: 100,
      userId: 42,
      userName: 'Requester',
      jobHeader: 'Target To Cancel',
      totalTracks: 5,
      statusMsgId: 88,
      controller: jobController,
      isCancelled: false,
      cachedCount: 0,
      rippedCount: 0,
      failedCount: 0,
      completed: false,
      queuePosition: 0,
      startTime: Date.now(),
    }
    activeJobs.set('cancel_me', job)

    registerStatusCommand(ctx)

    // Unauthorized non-admin, non-owner
    const unauthRes = await dispatchCallback(
      'status:cancel:cancel_me:1',
      99,
      100,
      200,
    )
    expect(unauthRes.answers[0].text).toContain(
      'You cannot cancel this download',
    )
    expect(job.isCancelled).toBe(false)
    expect(jobController.signal.aborted).toBe(false)

    // Job requester cancels
    const authRes = await dispatchCallback(
      'status:cancel:cancel_me:1',
      42,
      100,
      200,
    )
    expect(authRes.answers[0].text).toContain('Download cancelled')
    expect(job.isCancelled).toBe(true)
    expect(jobController.signal.aborted).toBe(true)
  })

  it('rejects unauthorized users from /status', async () => {
    mockAuth.isAuthorized = mock(() => Promise.resolve(false))
    registerStatusCommand(ctx)

    const { repliedTexts } = await dispatchMessage('/status', 999)
    expect(repliedTexts.length).toBe(0)
  })
})
