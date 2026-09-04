import type { PgliteDatabase } from 'drizzle-orm/pglite'
import {
  drizzle as drizzlePostgresJs,
  type PostgresJsDatabase,
} from 'drizzle-orm/postgres-js'
import postgres from 'postgres'

import { env } from '../env.ts'
import * as schema from './schema.ts'

export type AppDatabase =
  | PgliteDatabase<typeof schema>
  | PostgresJsDatabase<typeof schema>

export const isProduction = Boolean(env.DATABASE_URL)

async function createDb(): Promise<AppDatabase> {
  if (env.DATABASE_URL) {
    const client = postgres(env.DATABASE_URL)
    return drizzlePostgresJs(client, { schema })
  }

  const { PGlite } = await import('@electric-sql/pglite')
  const { drizzle: drizzlePglite } = await import('drizzle-orm/pglite')
  const client = new PGlite(env.DATABASE_DIR)
  return drizzlePglite(client, { schema })
}

export const db: AppDatabase = await createDb()
