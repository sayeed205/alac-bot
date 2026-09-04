import { describe, expect, it } from 'bun:test'

import { db, pool } from '@/db/index.ts'

describe('Database Client (index.ts)', () => {
  it('exposes the configured Drizzle db and pg pool instance', () => {
    expect(db.select).toBeDefined()
    expect(typeof db.select).toBe('function')
    expect(pool).toBeDefined()
  })
})
