import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import { type NewRequest, requests } from '@/db/schema.ts'
import { debugSpan } from '@/utils/logger.ts'

export interface IRequestLogRepository {
  logRequest(data: NewRequest): Promise<void>
}

export class RequestLogRepository implements IRequestLogRepository {
  private readonly _db?: AppDatabase

  constructor(db?: AppDatabase) {
    this._db = db
  }

  private get db(): AppDatabase {
    return this._db ?? defaultDb
  }

  async logRequest(data: NewRequest): Promise<void> {
    using _ = debugSpan('db_log_request', {
      appleTrackId: data.appleTrackId,
      status: data.status,
    }).enter()

    await this.db.insert(requests).values(data)
  }
}

export const requestLogRepository = new RequestLogRepository()
