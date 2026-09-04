import { sql } from 'drizzle-orm'

import { type AppDatabase, db as defaultDb } from '@/db/index.ts'
import * as schema from '@/db/schema.ts'

export interface DumpStats {
  usersCount: number
  tracksCount: number
  requestsCount: number
  bytes: number
}

export interface RestoreStats {
  usersMerged: number
  tracksMerged: number
  requestsMerged: number
  durationMs: number
}

export interface IDbDumpService {
  exportDump(): Promise<{
    buffer: Uint8Array
    stats: DumpStats
    filename: string
  }>
  importDump(gzipBuffer: Uint8Array): Promise<RestoreStats>
}

function escapeSqlString(str: string): string {
  return `'${str.replace(/'/g, "''")}'`
}

function escapeSqlValue(val: unknown): string {
  if (val === null || val === undefined) {
    return 'NULL'
  }
  if (typeof val === 'number') {
    return Number.isFinite(val) ? String(val) : 'NULL'
  }
  if (typeof val === 'boolean') {
    return val ? 'TRUE' : 'FALSE'
  }
  if (val instanceof Date) {
    return `'${val.toISOString()}'::timestamptz`
  }
  if (typeof val === 'string') {
    return escapeSqlString(val)
  }
  return escapeSqlString(String(val))
}

export class DbDumpService implements IDbDumpService {
  constructor(private readonly db: AppDatabase = defaultDb) {}

  async exportDump(): Promise<{
    buffer: Uint8Array
    stats: DumpStats
    filename: string
  }> {
    const allUsers = await this.db.select().from(schema.users)
    const allTracks = await this.db.select().from(schema.tracks)
    const allRequests = await this.db.select().from(schema.requests)

    const lines: string[] = []
    const now = new Date()
    lines.push('-- ALAC Telegram Bot Database Dump')
    lines.push(`-- Generated: ${now.toISOString()}`)
    lines.push('-- Format: SQL-GZ Transactional Upsert Dump')
    lines.push('')

    for (const u of allUsers) {
      const stmt = `INSERT INTO users (telegram_id, name, created_at) VALUES (${escapeSqlValue(u.telegramId)}, ${escapeSqlValue(u.name)}, ${escapeSqlValue(u.createdAt)}) ON CONFLICT (telegram_id) DO UPDATE SET name = EXCLUDED.name, created_at = LEAST(users.created_at, EXCLUDED.created_at);`
      lines.push(stmt)
    }

    for (const t of allTracks) {
      const stmt = `INSERT INTO tracks (apple_track_id, message_id, file_id, file_unique_id, title, artist, album, duration, bit_depth, sample_rate, genre, release_date, track_number, track_count, created_at, updated_at) VALUES (${escapeSqlValue(t.appleTrackId)}, ${escapeSqlValue(t.messageId)}, ${escapeSqlValue(t.fileId)}, ${escapeSqlValue(t.fileUniqueId)}, ${escapeSqlValue(t.title)}, ${escapeSqlValue(t.artist)}, ${escapeSqlValue(t.album)}, ${escapeSqlValue(t.duration)}, ${escapeSqlValue(t.bitDepth)}, ${escapeSqlValue(t.sampleRate)}, ${escapeSqlValue(t.genre)}, ${escapeSqlValue(t.releaseDate)}, ${escapeSqlValue(t.trackNumber)}, ${escapeSqlValue(t.trackCount)}, ${escapeSqlValue(t.createdAt)}, ${escapeSqlValue(t.updatedAt)}) ON CONFLICT (apple_track_id) DO UPDATE SET message_id = EXCLUDED.message_id, file_id = EXCLUDED.file_id, file_unique_id = EXCLUDED.file_unique_id, title = EXCLUDED.title, artist = EXCLUDED.artist, album = EXCLUDED.album, duration = EXCLUDED.duration, bit_depth = EXCLUDED.bit_depth, sample_rate = EXCLUDED.sample_rate, genre = EXCLUDED.genre, release_date = EXCLUDED.release_date, track_number = EXCLUDED.track_number, track_count = EXCLUDED.track_count, updated_at = GREATEST(tracks.updated_at, EXCLUDED.updated_at);`
      lines.push(stmt)
    }

    for (const r of allRequests) {
      const stmt = `INSERT INTO requests (telegram_id, chat_id, apple_track_id, is_cache_hit, duration_ms, status, error_reason, created_at) VALUES (${escapeSqlValue(r.telegramId)}, ${escapeSqlValue(r.chatId)}, ${escapeSqlValue(r.appleTrackId)}, ${escapeSqlValue(r.isCacheHit)}, ${escapeSqlValue(r.durationMs)}, ${escapeSqlValue(r.status)}, ${escapeSqlValue(r.errorReason)}, ${escapeSqlValue(r.createdAt)});`
      lines.push(stmt)
    }

    lines.push(
      "SELECT setval(pg_get_serial_sequence('tracks', 'id'), COALESCE((SELECT MAX(id) FROM tracks), 1), (SELECT MAX(id) IS NOT NULL FROM tracks));",
    )
    lines.push(
      "SELECT setval(pg_get_serial_sequence('requests', 'id'), COALESCE((SELECT MAX(id) FROM requests), 1), (SELECT MAX(id) IS NOT NULL FROM requests));",
    )

    const sqlContent = lines.join('\n')
    const compressed = Bun.gzipSync(Buffer.from(sqlContent))
    const timestampStr = now.toISOString().replace(/[:.]/g, '-').slice(0, 19)
    const filename = `alac_dump_${timestampStr}.sql.gz`

    return {
      buffer: compressed,
      filename,
      stats: {
        usersCount: allUsers.length,
        tracksCount: allTracks.length,
        requestsCount: allRequests.length,
        bytes: compressed.length,
      },
    }
  }

  async importDump(gzipBuffer: Uint8Array): Promise<RestoreStats> {
    const startTime = Date.now()
    const decompressed = Bun.gunzipSync(Buffer.from(gzipBuffer))
    const sqlText = new TextDecoder().decode(decompressed)

    const rawStatements = sqlText
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0 && !line.startsWith('--'))

    let usersMerged = 0
    let tracksMerged = 0
    let requestsMerged = 0

    for (const stmt of rawStatements) {
      if (stmt.startsWith('INSERT INTO users')) {
        usersMerged += 1
      } else if (stmt.startsWith('INSERT INTO tracks')) {
        tracksMerged += 1
      } else if (stmt.startsWith('INSERT INTO requests')) {
        requestsMerged += 1
      }
    }

    await this.db.transaction(async (tx) => {
      for (const stmt of rawStatements) {
        await tx.execute(sql.raw(stmt))
      }
    })

    return {
      usersMerged,
      tracksMerged,
      requestsMerged,
      durationMs: Date.now() - startTime,
    }
  }
}

export const dbDumpService = new DbDumpService()
