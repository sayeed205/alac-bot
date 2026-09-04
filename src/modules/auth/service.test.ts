import { afterAll, beforeAll, describe, expect, it } from 'bun:test'

import { PGlite } from '@electric-sql/pglite'
import { drizzle } from 'drizzle-orm/pglite'
import { migrate } from 'drizzle-orm/pglite/migrator'

import * as schema from '@/db/schema.ts'
import { AuthService } from '@/modules/auth/service.ts'

describe('AuthService', () => {
  let client: PGlite
  let authService: AuthService
  const ADMIN_ID = 6252490183
  const USER_A = 11111111
  const USER_B = 22222222
  const GROUP_A = -100123456789

  beforeAll(async () => {
    client = new PGlite()
    await client.waitReady
    const db = drizzle(client, { schema })
    await migrate(db, { migrationsFolder: './drizzle' })
    authService = new AuthService(db, ADMIN_ID)
  })

  afterAll(async () => {
    if (client && !client.closed) {
      await client.close()
    }
  })

  it('correctly identifies admin', () => {
    expect(authService.isAdmin(ADMIN_ID)).toBe(true)
    expect(authService.isAdmin(USER_A)).toBe(false)
  })

  it('admin is always authorized without being in the database', async () => {
    expect(await authService.isAuthorized(ADMIN_ID)).toBe(true)
  })

  it('unauthorized user returns false', async () => {
    expect(await authService.isAuthorized(USER_A)).toBe(false)
  })

  it('authorizes a new user', async () => {
    const res = await authService.authorize(USER_A, 'Alice')
    expect(res.newlyAdded).toBe(true)
    expect(await authService.isAuthorized(USER_A)).toBe(true)
  })

  it('updating an existing authorized user returns newlyAdded: false', async () => {
    const res = await authService.authorize(USER_A, 'Alice Updated')
    expect(res.newlyAdded).toBe(false)
    const list = await authService.listAuthorized()
    const found = list.find((u) => u.telegramId === USER_A)
    expect(found?.name).toBe('Alice Updated')
  })

  it('authorizes a group and verifies members inside the group', async () => {
    await authService.authorize(GROUP_A, 'Test Group')
    // USER_B is not directly authorized:\n    expect(await authService.isAuthorized(USER_B)).toBe(false)
    // But when checking inside authorized group GROUP_A:\n    expect(await authService.isAuthorized(USER_B, GROUP_A)).toBe(true)
  })

  it('lists all authorized users and groups', async () => {
    const list = await authService.listAuthorized()
    expect(list.length).toBe(2)
    expect(list.map((u) => u.telegramId)).toContain(USER_A)
    expect(list.map((u) => u.telegramId)).toContain(GROUP_A)
  })

  it('revokes an authorized user and group', async () => {
    const revUser = await authService.revoke(USER_A)
    expect(revUser.revoked).toBe(true)
    expect(await authService.isAuthorized(USER_A)).toBe(false)

    const revAgain = await authService.revoke(USER_A)
    expect(revAgain.revoked).toBe(false)

    const revGroup = await authService.revoke(GROUP_A)
    expect(revGroup.revoked).toBe(true)
    expect(await authService.isAuthorized(USER_B, GROUP_A)).toBe(false)

    const list = await authService.listAuthorized()
    expect(list.length).toBe(0)
  })
})
