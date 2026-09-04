import { TelegramClient } from '@mtcute/bun'
import { Dispatcher, filters } from '@mtcute/dispatcher'

import { pgliteClient } from '@/db/index.ts'
import { runMigrations } from '@/db/migrate.ts'
import { env } from '@/env.ts'
import { registerAlacCommands } from '@/modules/alac/index.ts'
import { registerAuthCommands } from '@/modules/auth/index.ts'
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

registerAuthCommands(dp, tg)
registerAlacCommands(dp, tg)

dp.onNewMessage(filters.start, async (msg) => {
  await msg.answerText('Hello, world!')
})

const shutdown = async () => {
  info('Shutting down bot...')
  try {
    await tg.destroy()
  } catch {}
  try {
    if (pgliteClient) {
      await pgliteClient.close()
    }
  } catch {}
  process.exit(0)
}

process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)

const user = await tg.start({ botToken: env.BOT_TOKEN })
info('Bot started successfully', {
  username: user.username,
  bot_id: user.id,
  dump_channel: env.DUMP_CHANNEL_ID,
})
