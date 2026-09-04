# ALAC Bot

A high-performance Telegram bot for downloading Apple Music lossless (ALAC) audio tracks, albums, and playlists with synchronized lyrics, embedded high-resolution artwork, and smart channel caching.

Built with [Bun](https://bun.sh), [@mtcute/bun](https://mtcute.dev), and [Drizzle ORM](https://orm.drizzle.team).

---

## Features

- **True Lossless Audio**: Streams native Apple Lossless Audio Codec (ALAC 16-bit / 24-bit up to 192kHz) directly from decryption mirrors.
- **Automatic Fallback Engine**: If the primary mirror encounters downtime or timeouts, the bot automatically fails over to a secondary wrapper or custom mirror without interrupting downloads.
- **Instant Dump Channel Caching**: Every ripped track is indexed with full metadata and stored in a private Telegram dump channel. Cache hits deliver in under 200ms without consuming mirror bandwidth.
- **Full Album & Playlist Support**:
  - Individual track URLs or bare track IDs.
  - Full album URLs (`https://music.apple.com/.../album/...`).
  - Apple Music playlists (`https://music.apple.com/.../playlist/...` or `pl.xxx` / `pl.u-xxx`).
  - Upload a `.txt` file containing multiple links for automated batch downloads.
  - Multi-line commands with multiple URLs.
- **Group Chat Friendly**: When triggered in a group chat, audio files are delivered directly to the user's private chat (DM) to eliminate spam, keeping only a live progress card in the group.
- **Interactive Task Cancellation**:
  - Live `[Cancel Download]` inline button attached to the progress card.
  - `/cancel` command to abort the user's current running task.
  - Permission-controlled: only the requester or a bot admin can cancel.
- **Synced Lyrics Embedding**: Automatically prefetches and embeds word-by-word synced lyrics (TTML -> Enhanced LRC) or line-synced LRC from Apple Music and LRCLIB.
- **High-Res Metadata & Artwork**: Tags every track using FFmpeg with embedded high-resolution album cover art, release date, genre, track/disc numbers, and explicit `[E]` flags.
- **Interactive Catalog Search**: `/search <query>` searches both cached tracks and Apple Music's catalog with interactive inline button results.
- **Access Control**: Granular user and group authorization system (`/auth`, `/revoke`, `/list`).
- **Resilient Mirror Architecture**: Automatic mirror manifest resolution, health checks, 30s connection timeout, 45s streaming chunk inactivity reset, and circuit breakers against mirror outages.
- **Database Backup & Restore**: Export and import compressed database snapshots (`.sql.gz`) directly via Telegram DM.

---

## Prerequisites

- [Bun](https://bun.sh) (v1.2 or later)
- [FFmpeg](https://ffmpeg.org) installed on system path (required for audio tagging and artwork embedding)
- [PostgreSQL](https://www.postgresql.org/) database (with `pg_trgm` extension for fuzzy search)
- **Telegram API Credentials**: `API_ID` & `API_HASH` from [my.telegram.org](https://my.telegram.org), plus a `BOT_TOKEN` from [@BotFather](https://t.me/BotFather)
- **Telegram Dump Channel**: A private channel where the bot is added as an administrator (to store and cache audio files)

---

## Quick Start

### 1. Clone & Install Dependencies

```bash
git clone https://github.com/sayeed205/alac-bot.git
cd alac-bot
bun install
```

### 2. Configure Environment

Copy the example configuration file:

```bash
cp .env.example .env
```

Edit `.env` with your credentials:

```env
API_ID=1234567
API_HASH=abcdef0123456789abcdef0123456789
BOT_TOKEN=1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ
ADMIN_ID=123456789
DUMP_CHANNEL_ID=-1001234567890

# PostgreSQL Connection
DATABASE_URL=postgresql://user:password@localhost:5432/alac_bot

# Logging (trace | debug | info | warn | error | critical)
LOG_LEVEL=info

# (Optional) Primary mirror overrides (defaults to dynamic manifest resolution)
# ALAC_MIRROR_URL=https://custom-mirror.example.com
# ALAC_API_KEY=ak_custom_api_key

# (Optional) Secondary wrapper fallback engine (defaults to local wrapper)
ALAC_WRAPPER_URL=http://127.0.0.1:12340
# ALAC_WRAPPER_API_KEY=ak_wrapper_key
```

### 3. Run Database Migrations

```bash
bun run db:migrate
```

### 4. Start the Bot

```bash
# Development mode (with file watching)
bun dev

# Production mode
bun start
```

---

## Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `API_ID` | Telegram API ID from [my.telegram.org](https://my.telegram.org) | *Required* |
| `API_HASH` | Telegram API Hash from [my.telegram.org](https://my.telegram.org) | *Required* |
| `BOT_TOKEN` | Bot token from [@BotFather](https://t.me/BotFather) | *Required* |
| `ADMIN_ID` | Telegram User ID of the bot owner | *Required* |
| `DUMP_CHANNEL_ID` | Channel ID (`-100...`) used to store cached audio files | *Required* |
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://admin:password@localhost:5432/alac_bot` |
| `LOG_LEVEL` | Tracing log level (`trace`, `debug`, `info`, `warn`, `error`) | `info` |
| `ALAC_MIRROR_URL` | Optional static mirror URL override | Dynamic manifest |
| `ALAC_API_KEY` | Optional static mirror API key override | Dynamic manifest |
| `ALAC_WRAPPER_URL` | Secondary decryption wrapper / mirror URL fallback | `http://127.0.0.1:12340` |
| `ALAC_WRAPPER_API_KEY` | Optional API key for wrapper URL | None |

---

## Decryption Mirrors & Wrapper Fallback Engine

The bot uses a dual-engine architecture to prevent download failures during public mirror downtime:

1. **Primary Decryption Mirror**: Automatically fetched and refreshed from the dynamic mirror manifest (or overridden with `ALAC_MIRROR_URL`).
2. **Wrapper Fallback Engine**: Configured via `ALAC_WRAPPER_URL` (defaults to the local wrapper at `http://127.0.0.1:12340`).

If the primary mirror is unreachable, returns HTTP 502/503, or drops the connection mid-handshake, the ripper seamlessly switches to the wrapper engine.

### Swapping the Wrapper Engine with Any Link

To swap the local wrapper with an alternative remote wrapper or third-party mirror in the future, simply update `ALAC_WRAPPER_URL` in `.env`:

```env
# Example 1: Local containerized wrapper
ALAC_WRAPPER_URL=http://127.0.0.1:12340

# Example 2: Remote private wrapper
ALAC_WRAPPER_URL=https://wrapper.yourdomain.com

# Example 3: Dedicated mirror with API key
ALAC_WRAPPER_URL=https://custom-mirror.example.com
ALAC_WRAPPER_API_KEY=your_secret_api_key
```

---

## Command Reference

### General Commands

| Command | Description |
| :--- | :--- |
| `/alac <url\|id> [-f]` | Download Apple Music track, album, or playlist in lossless ALAC (`-f` forces re-rip for admins) |
| `/rip`, `/batch`, `/dl`, `/download` | Aliases for `/alac` |
| `/search <query>` | Search Apple Music catalog and cached library with interactive buttons |
| `/info <url\|id>` | Show track metadata and cache availability |
| `/cancel` | Cancel your current active download |
| `/help` | Display usage instructions and available commands |

> **Batch Tip**: You can upload a `.txt` document containing one Apple Music link per line with `/alac` or `/batch` as the caption to rip entire batches automatically.

---

### Admin Management Commands

| Command | Description |
| :--- | :--- |
| `/auth [id\|username]` | Authorize a user or group to use the bot |
| `/revoke [id\|username]` | Revoke access from a user or group |
| `/list` | Show paginated list of authorized users and groups |
| `/health` | Check live latency and mirror wrapper instance health |
| `/stats` | View bot performance, queue metrics, and top requested tracks |
| `/queue` | View active and pending download tasks in the sequential queue |
| `/clean` | Clean up leftover temporary files in download scratch directory |
| `/delete <id>` | Remove a track from cache and the dump channel |
| `/index` | Re-index and synchronize existing tracks in the dump channel |
| `/export` | Export a compressed PostgreSQL database backup (`.sql.gz`) via DM |
| `/import` | Restore database by replying to a `.sql.gz` backup file |

---

## Testing & Code Quality

The repository includes a test suite covering parsing, iTunes integration, playlist scraping, sequential queue handling, ripper timeouts, wrapper failover, and database schema constraints.

```bash
# Run all tests
bun test

# Run linter and typecheck
bun run lint

# Auto-fix formatting and lint issues
bun run lint:fix
```

---

## Database Management

Database migrations are powered by Drizzle ORM:

```bash
bun run db:generate   # Generate migration SQL files from schema
bun run db:migrate    # Apply pending migrations to PostgreSQL
bun run db:push       # Push schema changes directly (dev prototyping)
bun run db:studio     # Launch Drizzle Studio web UI
```

---

## Credits

Special thanks and credit to the [applebruh](https://github.com/avikekkk/applebruh) project for inspiration and foundational research on Apple Music ALAC decryption and workflows.

---

## Disclaimer

This software is strictly intended for **educational, experimental, and research purposes only**.

- This project is **not affiliated with, associated with, authorized by, endorsed by, or in any way officially connected with Apple Inc.** or any of its subsidiaries or affiliates.
- "Apple", "Apple Music", and "ALAC" are registered trademarks of Apple Inc.
- Users are solely responsible for ensuring that their use of this software complies with all applicable local, national, and international laws, as well as the terms of service of any third-party platforms. The authors and maintainers assume no liability or responsibility for any misuse or violation of copyright or terms of service.

---

## License

This project is licensed under the [MIT License](LICENSE).
