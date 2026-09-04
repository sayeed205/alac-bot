import { eq, or } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { users } from '@/db/schema.ts'
import { env } from '@/env.ts'

export interface IAuthService {
  isAdmin(userId: number): boolean
  isAuthorized(userId: number, chatId?: number): Promise<boolean>
  authorize(
    telegramId: number,
    name?: string | null,
  ): Promise<{ newlyAdded: boolean }>
  revoke(telegramId: number): Promise<{ revoked: boolean }>
  listAuthorized(): Promise<
    Array<{ telegramId: number; name: string | null; createdAt: Date }>
  >
}

export class AuthService implements IAuthService {
  private readonly _db?: AppDatabase

  constructor(
    db?: AppDatabase,
    private readonly adminId: number = env.ADMIN_ID,
  ) {
    this._db = db
  }

  private get db(): AppDatabase {
    return this._db ?? defaultDb
  }

  isAdmin(userId: number): boolean {
    return userId === this.adminId
  }

  async isAuthorized(userId: number, chatId?: number): Promise<boolean> {
    if (this.isAdmin(userId)) {
      return true
    }

    const conditions = [eq(users.telegramId, userId)]
    if (chatId && chatId !== userId) {
      conditions.push(eq(users.telegramId, chatId))
    }

    const matched = await this.db
      .select({ telegramId: users.telegramId })
      .from(users)
      .where(conditions.length === 1 ? conditions[0] : or(...conditions))
      .limit(1)

    return matched.length > 0
  }

  async authorize(
    telegramId: number,
    name?: string | null,
  ): Promise<{ newlyAdded: boolean }> {
    const existing = await this.db
      .select({ telegramId: users.telegramId })
      .from(users)
      .where(eq(users.telegramId, telegramId))
      .limit(1)

    await this.db
      .insert(users)
      .values({
        telegramId,
        name: name ?? null,
      })
      .onConflictDoUpdate({
        target: users.telegramId,
        set: {
          name: name ?? null,
        },
      })

    return { newlyAdded: existing.length === 0 }
  }

  async revoke(telegramId: number): Promise<{ revoked: boolean }> {
    const deleted = await this.db
      .delete(users)
      .where(eq(users.telegramId, telegramId))
      .returning()

    return { revoked: deleted.length > 0 }
  }

  async listAuthorized(): Promise<
    Array<{ telegramId: number; name: string | null; createdAt: Date }>
  > {
    return this.db
      .select({
        telegramId: users.telegramId,
        name: users.name,
        createdAt: users.createdAt,
      })
      .from(users)
      .orderBy(users.createdAt)
  }
}

export const authService = new AuthService()
