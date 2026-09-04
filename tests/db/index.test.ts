import { describe, expect, it } from 'bun:test'

import { db, isProduction, pgliteClient } from '@/db/index.ts'

describe('Database Client & Proxy (index.ts)', () => {
  it('exposes isProduction boolean based on env', () => {
    expect(typeof isProduction).toBe('boolean')
  })

  it('proxies queries through getOrCreateDb to the active database instance', () => {
    expect(db.select).toBeDefined()
    expect(typeof db.select).toBe('function')

    if (!isProduction) {
      expect(pgliteClient).toBeDefined()
    }
  })
})
