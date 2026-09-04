import {
  bigint,
  boolean,
  index,
  integer,
  pgTable,
  serial,
  text,
  timestamp,
} from 'drizzle-orm/pg-core'

export const users = pgTable('users', {
  telegramId: bigint('telegram_id', { mode: 'number' }).primaryKey(),
  name: text('name'),
  createdAt: timestamp('created_at', { withTimezone: true })
    .defaultNow()
    .notNull(),
})

export const tracks = pgTable(
  'tracks',
  {
    id: serial('id').primaryKey(),
    appleTrackId: text('apple_track_id').notNull().unique(),
    messageId: integer('message_id').notNull(),
    fileId: text('file_id').notNull(),
    fileUniqueId: text('file_unique_id').notNull(),
    title: text('title').notNull(),
    artist: text('artist').notNull(),
    album: text('album').notNull(),
    duration: integer('duration').notNull(),
    bitDepth: integer('bit_depth').notNull(),
    sampleRate: integer('sample_rate').notNull(),
    genre: text('genre').notNull(),
    releaseDate: text('release_date').notNull(),
    trackNumber: integer('track_number').notNull(),
    trackCount: integer('track_count').notNull(),
    createdAt: timestamp('created_at', { withTimezone: true })
      .defaultNow()
      .notNull(),
    updatedAt: timestamp('updated_at', { withTimezone: true })
      .defaultNow()
      .notNull(),
  },
  (table) => [
    index('tracks_title_idx').on(table.title),
    index('tracks_artist_idx').on(table.artist),
  ],
)

export const requests = pgTable(
  'requests',
  {
    id: serial('id').primaryKey(),
    telegramId: bigint('telegram_id', { mode: 'number' }).notNull(),
    chatId: bigint('chat_id', { mode: 'number' }).notNull(),
    appleTrackId: text('apple_track_id').notNull(),
    isCacheHit: boolean('is_cache_hit').notNull(),
    durationMs: integer('duration_ms'),
    status: text('status').notNull(),
    errorReason: text('error_reason'),
    createdAt: timestamp('created_at', { withTimezone: true })
      .defaultNow()
      .notNull(),
  },
  (table) => [
    index('requests_apple_track_id_idx').on(table.appleTrackId),
    index('requests_telegram_id_idx').on(table.telegramId),
    index('requests_created_at_idx').on(table.createdAt),
  ],
)

export type User = typeof users.$inferSelect
export type NewUser = typeof users.$inferInsert

export type Track = typeof tracks.$inferSelect
export type NewTrack = typeof tracks.$inferInsert

export type Request = typeof requests.$inferSelect
export type NewRequest = typeof requests.$inferInsert
