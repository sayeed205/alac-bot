import { TelegramClient } from '@mtcute/bun'
import { Dispatcher, filters } from '@mtcute/dispatcher'

import { runMigrations } from '@/db/migrate.ts'
import { env } from '@/env.ts'
import { registerAlacHandlers } from '@/modules/alac/index.ts'
import { registerAuthHandlers } from '@/modules/auth/index.ts'
import { info, infoSpan, initTracing } from '@/utils/logger.ts'

initTracing(env.LOG_LEVEL)

using _startupSpan = infoSpan('startup').enter()

info('Running database migrations...')
await runMigrations()

info('Initializing Telegram client...')
const tg = new TelegramClient({
  apiId: env.API_ID,
  apiHash: env.API_HASH,
  storage: 'bot-data/session',
})

const dp = Dispatcher.for(tg)

registerAuthHandlers(dp, tg)
registerAlacHandlers(dp, tg)

dp.onNewMessage(filters.start, async (msg) => {
  await msg.answerText('Hello, world!')
})

const user = await tg.start({ botToken: env.BOT_TOKEN })
info('Bot started successfully', {
  username: user.username,
  bot_id: user.id,
  dump_channel: env.DUMP_CHANNEL_ID,
})
