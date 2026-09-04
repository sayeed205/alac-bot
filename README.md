# alac-bot

mtcute-powered Telegram bot with Drizzle ORM and Biome.

## Database Strategy

- **Local Development**: Uses embedded **PGlite** by default (runs PostgreSQL in-process, stored in `./bot-data/db`). Zero external server or Docker setup needed, exactly like SQLite, but fully PostgreSQL-compatible.
- **Production**: Set `DATABASE_URL=postgres://user:password@host:5432/db` in `.env`. The bot automatically connects to PostgreSQL using the exact same schema and migrations.

## Getting Started

```bash
bun install
cp .env.example .env
# Edit .env with your Telegram credentials (API_ID, API_HASH, BOT_TOKEN)
bun dev
```

## Database Commands

- `bun run db:migrate` - Run migrations against current environment (PGlite locally, or PostgreSQL if `DATABASE_URL` is set)
- `bun run db:generate` - Generate new SQL migration files from `src/db/schema.ts`
- `bun run db:push` - Push schema changes directly to the database
- `bun run db:studio` - Launch Drizzle Studio interface

## Linting & Formatting

```bash
bun run lint      # Check formatting and lint rules
bun run lint:fix  # Apply auto-fixes
bun run format    # Format files
```
