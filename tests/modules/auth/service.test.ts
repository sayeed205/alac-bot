import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'bun:test'

import type { AppDatabase } from '@/db/index.ts'
import { AuthService } from '@/modules/auth/service.ts'

import { setupTestDb } from '../../test-db.ts'

describe('AuthService', () => {
  let db: AppDatabase
  let cleanDb: () => Promise<void>
  let close: () => Promise<void>
  let authService: AuthService
  const ADMIN_ID = 6252490183
  const USER_A = 11111111
  const USER_B = 22222222
  const GROUP_A = -100123456789

  beforeAll(async () => {
    const testEnv = await setupTestDb()
    db = testEnv.db
    cleanDb = testEnv.cleanDb
    close = testEnv.close
    authService = new AuthService(db, ADMIN_ID)
  })

  beforeEach(async () => {
    await cleanDb()
  })

  afterAll(async () => {
    await close()
  })

  it('correctly identifies admin', () => {
    expect(authService.isAdmin(ADMIN_ID)).toBe(true)
    expect(authService.isAdmin(USER_A)).toBe(false)
  })

  it('admin is always authorized without being in the database', async () => {
    const isAuth = await authService.isAuthorized(ADMIN_ID)
    expect(isAuth).toBe(true)
  })

  it('unauthorized user returns false', async () => {
    const isAuth = await authService.isAuthorized(USER_A)
    expect(isAuth).toBe(false)
  })

  it('authorizes a new user', async () => {
    const result = await authService.authorize(USER_A, 'User A')
    expect(result.newlyAdded).toBe(true)

    const isAuth = await authService.isAuthorized(USER_A)
    expect(isAuth).toBe(true)
  })

  it('updating an existing authorized user returns newlyAdded: false', async () => {
    await authService.authorize(USER_A, 'User A')
    const result = await authService.authorize(USER_A, 'User A Updated')
    expect(result.newlyAdded).toBe(false)

    const list = await authService.listAuthorized()
    expect(list.length).toBe(1)
    expect(list[0]?.name).toBe('User A Updated')
  })

  it('authorizes a group and verifies members inside the group', async () => {
    const result = await authService.authorize(GROUP_A, 'Test Group')
    expect(result.newlyAdded).toBe(true)

    // Inside group chat, isAuthorized should return true
    const isAuthInGroup = await authService.isAuthorized(USER_A, GROUP_A)
    expect(isAuthInGroup).toBe(true)

    // Outside group chat (in private DM), USER_A should still not be authorized
    const isAuthPrivate = await authService.isAuthorized(USER_A)
    expect(isAuthPrivate).toBe(false)
  })

  it('lists all authorized users and groups', async () => {
    await authService.authorize(USER_A, 'User A')
    await authService.authorize(USER_B, 'User B')
    await authService.authorize(GROUP_A, 'Test Group')

    const list = await authService.listAuthorized()
    expect(list.length).toBe(3)
  })

  it('revokes an authorized user and group', async () => {
    await authService.authorize(USER_A, 'User A')
    await authService.authorize(GROUP_A, 'Test Group')

    const userRevoked = await authService.revoke(USER_A)
    expect(userRevoked.revoked).toBe(true)
    expect(await authService.isAuthorized(USER_A)).toBe(false)

    const groupRevoked = await authService.revoke(GROUP_A)
    expect(groupRevoked.revoked).toBe(true)
    expect(await authService.isAuthorized(USER_B, GROUP_A)).toBe(false)

    // Revoking non-existent target returns false
    const nonExistentRevoked = await authService.revoke(99999999)
    expect(nonExistentRevoked.revoked).toBe(false)
  })
})
