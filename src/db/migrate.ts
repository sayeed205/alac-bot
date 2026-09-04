import fs from 'node:fs'

import type { PGlite } from '@electric-sql/pglite'
import type { PgliteDatabase } from 'drizzle-orm/pglite'
import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'

import { type AppDatabase, db, isProduction, pgliteClient } from '@/db/index.ts'
import type * as schema from '@/db/schema.ts'
import { info, warn } from '@/utils/logger.ts'

export async function runMigrations(
  targetDb?: AppDatabase,
  targetPglite?: PGlite | null,
) {
  if (!fs.existsSync('./drizzle/meta/_journal.json')) {
    warn(
      'No migrations found in ./drizzle folder. Run `bun run db:generate` after defining your schema.',
    )
    return
  }

  const activeDb = targetDb ?? db
  // Ensure the underlying database instance is initialized
  void (activeDb as unknown as { _instance?: unknown })._instance
  const activePglite = targetPglite !== undefined ? targetPglite : pgliteClient

  info(
    `Running migrations (${isProduction ? 'PostgreSQL' : 'PGlite local'})...`,
  )
  if (isProduction) {
    const { migrate } = await import('drizzle-orm/postgres-js/migrator')
    await migrate(activeDb as PostgresJsDatabase<typeof schema>, {
      migrationsFolder: './drizzle',
    })
  } else {
    if (activePglite) {
      await activePglite.waitReady
    }
    const { migrate } = await import('drizzle-orm/pglite/migrator')
    await migrate(activeDb as PgliteDatabase<typeof schema>, {
      migrationsFolder: './drizzle',
    })
  }
  info('Migrations completed successfully!')
}

if (import.meta.main) {
  await runMigrations()
  if (pgliteClient) {
    await pgliteClient.close()
  }
  process.exit(0)
}
