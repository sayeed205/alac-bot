import type { TelegramClient } from '@mtcute/bun'
import { html } from '@mtcute/bun'
import type { Dispatcher } from '@mtcute/dispatcher'

import type { IDbDumpService } from '@/db/dump.ts'
import type { IAuthService } from '@/modules/auth/service.ts'

import type { IRipQueue } from '../queue.ts'
import type { ITrackRipper } from '../ripper.ts'
import type { IAlacService } from '../service.ts'

export type FormattedString = ReturnType<typeof html>

export function parseDynamicHtml(content: string): FormattedString {
  return html([content] as unknown as TemplateStringsArray)
}

export interface CommandContext {
  dp: Dispatcher
  tg: TelegramClient
  service: IAlacService
  ripper: ITrackRipper
  queue: IRipQueue
  auth: IAuthService
  dumpService?: IDbDumpService
}
