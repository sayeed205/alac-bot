import { afterEach, describe, expect, it, mock } from 'bun:test'
import fs from 'node:fs'

import { runMigrations } from '@/db/migrate.ts'

describe('Database Migrations (migrate.ts)', () => {
  const originalExistsSync = fs.existsSync

  afterEach(() => {
    fs.existsSync = originalExistsSync
  })

  it('executes migrations successfully on existing journal', async () => {
    expect(runMigrations()).resolves.toBeUndefined()
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
