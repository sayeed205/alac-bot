import { PGlite } from '@electric-sql/pglite'
import {
  drizzle as drizzlePglite,
  type PgliteDatabase,
} from 'drizzle-orm/pglite'
import {
  drizzle as drizzlePostgresJs,
  type PostgresJsDatabase,
} from 'drizzle-orm/postgres-js'
import postgres from 'postgres'

import * as schema from '@/db/schema.ts'
import { env } from '@/env.ts'

export type AppDatabase =
  | PgliteDatabase<typeof schema>
  | PostgresJsDatabase<typeof schema>

export const isProduction = Boolean(env.DATABASE_URL)

let _instance: AppDatabase | null = null
export let pgliteClient: PGlite | null = null

function getOrCreateDb(): AppDatabase {
  if (!_instance) {
    if (env.DATABASE_URL) {
      const client = postgres(env.DATABASE_URL)
      _instance = drizzlePostgresJs(client, { schema })
    } else {
      const dbPath =
        process.env.NODE_ENV === 'test' ? undefined : env.DATABASE_DIR
      pgliteClient = new PGlite(dbPath)
      _instance = drizzlePglite(pgliteClient, { schema })
    }
  }
  return _instance
}

export const db: AppDatabase = new Proxy({} as AppDatabase, {
  get(_target, prop, receiver) {
    const instance = getOrCreateDb()
    return Reflect.get(instance, prop, receiver)
  },
})
