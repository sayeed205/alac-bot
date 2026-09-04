import fs from 'node:fs'

import { migrate } from 'drizzle-orm/node-postgres/migrator'

import { type AppDatabase, closeDb, db } from '@/db/index.ts'
import { info, warn } from '@/utils/logger.ts'

export async function runMigrations(targetDb?: AppDatabase) {
  if (!fs.existsSync('./drizzle/meta/_journal.json')) {
    warn(
      'No migrations found in ./drizzle folder. Run `bun run db:generate` after defining your schema.',
    )
    return
  }

  const activeDb = targetDb ?? db

  info('Running migrations (PostgreSQL)...')
  await migrate(activeDb, {
    migrationsFolder: './drizzle',
  })
  info('Migrations completed successfully!')
}

if (import.meta.main) {
  await runMigrations()
  await closeDb()
  process.exit(0)
}
