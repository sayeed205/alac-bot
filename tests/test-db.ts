import { drizzle } from 'drizzle-orm/node-postgres'
import { migrate } from 'drizzle-orm/node-postgres/migrator'
import { Pool } from 'pg'

import type { AppDatabase } from '@/db/index.ts'
import * as schema from '@/db/schema.ts'

export const TEST_DB_URL =
  process.env.TEST_DATABASE_URL ||
  'postgresql://admin:password@localhost:5432/alac_bot_test'

export async function setupTestDb(): Promise<{
  db: AppDatabase
  pool: Pool
  cleanDb: () => Promise<void>
  close: () => Promise<void>
}> {
  const pool = new Pool({
    connectionString: TEST_DB_URL,
  })
  const db = drizzle({ client: pool, schema })

  await migrate(db, { migrationsFolder: './drizzle' })

  const cleanDb = async () => {
    await pool.query(
      'TRUNCATE TABLE tracks, requests, users, settings RESTART IDENTITY CASCADE;',
    )
  }

  const close = async () => {
    await pool.end()
  }

  return { db, pool, cleanDb, close }
}
