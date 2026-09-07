import type { TelegramClient } from '@mtcute/bun'
import type { Dispatcher } from '@mtcute/dispatcher'

import type { IDbDumpService } from '@/db/dump.ts'
import type { IAuthService } from '@/modules/auth/service.ts'
import type { ISettingsService } from '@/modules/settings/service.ts'
import type { FormattedString } from '@/utils/html.ts'
import { parseDynamicHtml } from '@/utils/html.ts'

import type { IRipQueue } from '../queue.ts'
import type {
  IRequestLogRepository,
  IStatsRepository,
  ITrackRepository,
} from '../repositories/index.ts'
import type { ITrackRipper } from '../ripper.ts'
import type { IAlacService } from '../service.ts'

export type { FormattedString }
export { parseDynamicHtml }

export interface CommandContext {
  dp: Dispatcher
  tg: TelegramClient
  service: IAlacService
  tracks?: ITrackRepository
  requestLogs?: IRequestLogRepository
  stats?: IStatsRepository
  ripper: ITrackRipper
  queue: IRipQueue
  auth: IAuthService
  dumpService?: IDbDumpService
  settings?: ISettingsService
  uploadRetryBaseMs?: number
}
