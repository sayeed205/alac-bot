import type { TelegramClient } from '@mtcute/bun'
import type { Dispatcher } from '@mtcute/dispatcher'

import {
  ripQueue as defaultQueue,
  type IRipQueue,
} from '@/modules/alac/queue.ts'
import { defaultRipper, type ITrackRipper } from '@/modules/alac/ripper.ts'
import {
  alacService as defaultService,
  type IAlacService,
} from '@/modules/alac/service.ts'
import { authService, type IAuthService } from '@/modules/auth/service.ts'
import {
  settingsService as defaultSettingsService,
  type ISettingsService,
} from '@/modules/settings/service.ts'

import { registerBackupCommands } from './backup.ts'
import { registerCleanCommand } from './clean.ts'
import { registerDeleteCommand } from './delete.ts'
import { registerHealthCommand } from './health.ts'
import { registerHelpCommand } from './help.ts'
import { registerIndexCommand } from './index_cmd.ts'
import { registerInfoCommand } from './info.ts'
import { registerQueueCommand } from './queue.ts'
import { registerRandomCommand } from './random.ts'
import { registerReportCommand } from './report.ts'
import { registerRipCommand } from './rip.ts'
import { registerSearchCommand } from './search.ts'
import { registerSpecCommand } from './spec.ts'
import { registerStatsCommand } from './stats.ts'
import type { CommandContext } from './types.ts'

export * from './types.ts'

export function registerAlacCommands(ctx: CommandContext): void
export function registerAlacCommands(
  dp: Dispatcher,
  tg: TelegramClient,
  service?: IAlacService,
  ripper?: ITrackRipper,
  queue?: IRipQueue,
  auth?: IAuthService,
  settings?: ISettingsService,
): void
export function registerAlacCommands(
  dpOrCtx: Dispatcher | CommandContext,
  tg?: TelegramClient,
  service: IAlacService = defaultService,
  ripper: ITrackRipper = defaultRipper,
  queue: IRipQueue = defaultQueue,
  auth: IAuthService = authService,
  settings: ISettingsService = defaultSettingsService,
): void {
  const ctx: CommandContext =
    'dp' in dpOrCtx
      ? dpOrCtx
      : {
          dp: dpOrCtx,
          tg: tg as TelegramClient,
          service,
          ripper,
          queue,
          auth,
          settings,
        }

  registerHelpCommand(ctx)
  registerHealthCommand(ctx)
  registerInfoCommand(ctx)
  registerQueueCommand(ctx)
  registerCleanCommand(ctx)
  registerDeleteCommand(ctx)
  registerStatsCommand(ctx)
  registerIndexCommand(ctx)
  registerSearchCommand(ctx)
  registerRipCommand(ctx)
  registerRandomCommand(ctx)
  registerSpecCommand(ctx)
  registerReportCommand(ctx)
  registerBackupCommands(ctx)
}
