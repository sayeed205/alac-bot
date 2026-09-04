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

import { registerCleanCommand } from './clean.ts'
import { registerDeleteCommand } from './delete.ts'
import { registerHealthCommand } from './health.ts'
import { registerHelpCommand } from './help.ts'
import { registerIndexCommand } from './index_cmd.ts'
import { registerInfoCommand } from './info.ts'
import { registerQueueCommand } from './queue.ts'
import { registerRipCommand } from './rip.ts'
import { registerSearchCommand } from './search.ts'
import { registerStatsCommand } from './stats.ts'
import type { CommandContext } from './types.ts'

export * from './types.ts'

export function registerAlacCommands(ctx: CommandContext): void
export function registerAlacCommands(
  dp: Dispatcher<TelegramClient>,
  tg: TelegramClient,
  service?: IAlacService,
  ripper?: ITrackRipper,
  queue?: IRipQueue,
  auth?: IAuthService,
): void
export function registerAlacCommands(
  dpOrCtx: Dispatcher<TelegramClient> | CommandContext,
  tg?: TelegramClient,
  service: IAlacService = defaultService,
  ripper: ITrackRipper = defaultRipper,
  queue: IRipQueue = defaultQueue,
  auth: IAuthService = authService,
): void {
  const ctx: CommandContext =
    'dp' in dpOrCtx
      ? dpOrCtx
      : {
          dp: dpOrCtx,
          tg: tg ?? dpOrCtx.client,
          service,
          ripper,
          queue,
          auth,
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
}
