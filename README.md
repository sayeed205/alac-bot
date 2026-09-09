# ALAC Bot

A high-performance Telegram bot for downloading Apple Music lossless (ALAC) audio tracks, albums, and playlists with synchronized lyrics, embedded high-resolution artwork, and smart channel caching.

Built in Rust: [ferogram](https://github.com/ankit-chaubey/ferogram) (Telegram MTProto), tokio, and Diesel/PostgreSQL.

> **Migration note**: this codebase is the Rust port of the original Bun/TypeScript bot. The TypeScript implementation is preserved on the `typescript` branch; the migration history and per-milestone parity records live in [docs/](docs/).

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
- **Interactive Catalog Search**: `/search <query>` searches both cached tracks (with pg_trgm fuzzy matching) and Apple Music's catalog with interactive inline button results.
- **Access Control**: Granular user and group authorization system (`/auth`, `/revoke`, `/authlist`).
- **Resilient Mirror Architecture**: Automatic mirror manifest resolution, health checks, 30s connection timeout, 45s streaming chunk inactivity reset, and circuit breakers against mirror outages.
- **Auto-Dump Scheduler**: Daily 24h sweep that discovers new Apple Music releases and archives them straight to the dump channel.
- **Database Backup & Restore**: Export and import compressed, versioned database archives (`.json.gz`) directly via Telegram DM.

---

## Prerequisites

- [Rust](https://rustup.rs) (stable toolchain; nightly is only needed for `just fmt`)
- [just](https://github.com/casey/just) (task runner)
- [FFmpeg](https://ffmpeg.org) installed on system path (required for audio tagging and artwork embedding)
- [SoX](http://sox.sourceforge.net) with `libsox-fmt-all` (required for `/spec` spectrograms)
- [PostgreSQL](https://www.postgresql.org/) database (with `pg_trgm` extension for fuzzy search — installed automatically by the bot's migrations)
- **Telegram API Credentials**: `API_ID` & `API_HASH` from [my.telegram.org](https://my.telegram.org), plus a `BOT_TOKEN` from [@BotFather](https://t.me/BotFather)
- **Telegram Dump Channel**: A private channel where the bot is added as an administrator (to store and cache audio files)

---

## Quick Start

### 1. Clone & Configure

```bash
git clone https://github.com/sayeed205/alac-bot.git
cd alac-bot
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

# Logging (trace | debug | info | warn | error)
LOG_LEVEL=info

# (Optional) Primary mirror overrides (defaults to dynamic manifest resolution)
# ALAC_MIRROR_URL=https://custom-mirror.example.com
# ALAC_API_KEY=ak_custom_api_key

# (Optional) Secondary wrapper fallback engine (defaults to local wrapper)
ALAC_WRAPPER_URL=http://127.0.0.1:12340
# ALAC_WRAPPER_API_KEY=ak_wrapper_key
```

### 2. Start the Bot

The canonical database migration runs automatically at startup for a fresh
database. Existing databases are not upgraded or adopted; reset the database
before starting the bot when changing schema generations.

```bash
# Development (debug build)
just run

# Or release mode
just release && ./target/release/bot
```

### 3. Docker

```bash
docker build -t alac-bot .
docker run -d --name alac-bot \
  --env-file .env \
  -v alac-bot-data:/app/bot-data \
  alac-bot
```

The container ships ffmpeg/sox, runs as a non-root user, and persists the
Telegram session + download scratch in the `/app/bot-data` volume.

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
| `ALAC_MAX_RETRIES` | Max retries per rip/upload attempt | `3` |
| `ALAC_RETRY_BASE_MS` | Exponential backoff base delay (ms) | `2000` |

---

## Decryption Mirrors & Wrapper Fallback Engine

The bot uses a dual-engine architecture to prevent download failures during public mirror downtime:

1. **Primary Decryption Mirror**: Automatically fetched and refreshed from the dynamic mirror manifest (or overridden with `ALAC_MIRROR_URL`).
2. **Wrapper Fallback Engine**: Configured via `ALAC_WRAPPER_URL` (defaults to the local wrapper at `http://127.0.0.1:12340`).

If the primary mirror is unreachable, returns HTTP 502/503, or drops the connection mid-handshake, the ripper seamlessly switches to the wrapper engine.

### Swapping the Wrapper Engine with Any Link

To swap the local wrapper with an alternative remote wrapper or third-party mirror, simply update `ALAC_WRAPPER_URL` in `.env`:

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
| `/authlist` | Show paginated list of authorized users and groups |
| `/health`, `/ping` | Check live latency and mirror wrapper instance health |
| `/stats` | View bot performance, queue metrics, and top requested tracks |
| `/queue` | View active and pending download tasks in the sequential queue |
| `/status` | Live download status & queue dashboard |
| `/clean` | Clean up leftover temporary files in download scratch directory |
| `/delete <id>` | Remove a track from cache and the dump channel |
| `/index` | Re-index and synchronize existing tracks in the dump channel |
| `/spec` | Reply to audio to generate a frequency spectrogram (aliases: `/spectogram`, `/spectrogram`, `/spek`) |
| `/report`, `/issue` | Report a corrupt track to the admin for a re-rip |
| `/settings` | Bot operational settings & ripping toggles |
| `/dumpnew <days>` / `/autodump` | Auto-dump new releases from Apple Music (also runs daily on a 24h scheduler) |
| `/cache <link>` / `/dump` | Pre-cache/seed tracks directly into dump channel without sending audio |
| `/random` | Interactive random album discovery & dump |
| `/export` | Export a compressed PostgreSQL database archive (`.json.gz`) via DM |
| `/import` | Restore database by replying to a `.json.gz` archive file |

---

## Development

Task recipes live in [justfile](justfile) — `just` with no arguments lists them.

```bash
just fmt          # format (nightly rustfmt: import grouping/sorting)
just fmt-check    # CI-style format verification
just check        # fast workspace type-check
just clippy       # lint with warnings as errors
just test         # test suite (needs PostgreSQL; set DATABASE_URL)
just build        # debug build
just release      # optimized build
just run          # build + run with .env
just docker       # build the container image
```

Toolchains: **stable** for build/lint/test — **nightly** only for `fmt`,
because rustfmt's import grouping/sorting (see `rustfmt.toml`) is a
nightly-only option.

Testing requires a PostgreSQL database with the pg_trgm extension
available:

```bash
export DATABASE_URL=postgresql://admin:password@localhost:5432/alac_bot_v2_test
just test
```

---

## Database Management

The canonical Diesel migration is embedded in the binary and runs
automatically at startup (`db::migrate`) — no separate migration step is
needed. This is a clean-slate schema: it intentionally provides no upgrade
path from older database schemas.

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

This project is licensed under the MIT License.
