import fs from 'node:fs'

import type { PgliteDatabase } from 'drizzle-orm/pglite'
import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'

import { db, isProduction, pgliteClient } from '@/db/index.ts'
import type * as schema from '@/db/schema.ts'
import { info, warn } from '@/utils/logger.ts'

export async function runMigrations() {
  if (!fs.existsSync('./drizzle/meta/_journal.json')) {
    warn(
      'No migrations found in ./drizzle folder. Run `bun run db:generate` after defining your schema.',
    )
    return
  }

  info(
    `Running migrations (${isProduction ? 'PostgreSQL' : 'PGlite local'})...`,
  )
  if (isProduction) {
    const { migrate } = await import('drizzle-orm/postgres-js/migrator')
    await migrate(db as PostgresJsDatabase<typeof schema>, {
      migrationsFolder: './drizzle',
    })
  } else {
    void db
    if (pgliteClient) {
      await pgliteClient.waitReady
    }
    const { migrate } = await import('drizzle-orm/pglite/migrator')
    await migrate(db as PgliteDatabase<typeof schema>, {
      migrationsFolder: './drizzle',
    })
  }
  info('Migrations completed successfully!')
}

if (import.meta.main) {
  await runMigrations()
  process.exit(0)
}
