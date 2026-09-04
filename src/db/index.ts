import { drizzle } from 'drizzle-orm/node-postgres'
import { Pool } from 'pg'

import * as schema from '@/db/schema.ts'
import { env } from '@/env.ts'

export const pool = new Pool({
  connectionString: env.DATABASE_URL,
})

export const db = drizzle({ client: pool, schema })
export type AppDatabase = typeof db

export async function closeDb(): Promise<void> {
  await pool.end()
}
