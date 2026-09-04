import { afterEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'

import { PGlite } from '@electric-sql/pglite'
import { drizzle } from 'drizzle-orm/pglite'

import { runMigrations } from '@/db/migrate.ts'
import * as schema from '@/db/schema.ts'

describe('Database Migrations (migrate.ts)', () => {
  const originalExistsSync = fs.existsSync

  afterEach(() => {
    fs.existsSync = originalExistsSync
  })

  it('executes migrations successfully on existing journal', async () => {
    const client = new PGlite()
    await client.waitReady
    const testDb = drizzle(client, { schema })
    await expect(runMigrations(testDb, client)).resolves.toBeUndefined()
    await client.close()
  })

  it('warns and exits early when journal is missing', async () => {
    fs.existsSync = mock((p: fs.PathLike) => {
      if (String(p).includes('_journal.json')) {
        return false
      }
      return originalExistsSync(p)
    })

    await expect(runMigrations()).resolves.toBeUndefined()
  })
})
