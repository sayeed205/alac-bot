import type { TelegramClient } from '@mtcute/bun'
import type { Dispatcher } from '@mtcute/dispatcher'

import { authService, type IAuthService } from '@/modules/auth/service.ts'

import { registerAuthCommand } from './auth.ts'
import { registerListCommand } from './list.ts'
import { registerRevokeCommand } from './revoke.ts'
import type { CommandContext } from './types.ts'

export * from './types.ts'

export function registerAuthCommands(ctx: CommandContext): void
export function registerAuthCommands(
  dp: Dispatcher<TelegramClient>,
  tg: TelegramClient,
  service?: IAuthService,
): void
export function registerAuthCommands(
  dpOrCtx: Dispatcher<TelegramClient> | CommandContext,
  tg?: TelegramClient,
  service: IAuthService = authService,
): void {
  const ctx: CommandContext =
    'dp' in dpOrCtx
      ? dpOrCtx
      : {
          dp: dpOrCtx,
          tg: tg ?? dpOrCtx.client,
          service,
        }

  registerAuthCommand(ctx)
  registerRevokeCommand(ctx)
  registerListCommand(ctx)
}
