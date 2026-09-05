# syntax=docker/dockerfile:1

# ==============================================================================
# Stage 1: Base with system dependencies (ffmpeg, sox, certificates)
# ==============================================================================
FROM docker.io/oven/bun:1-slim AS base

# Install system dependencies:
# - ffmpeg / ffprobe: audio metadata probing, tagging, and cover art embedding
# - sox / libsox-fmt-all: audio spectrogram generation (/spec and /spectrogram)
# - ca-certificates: TLS root certificates
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    sox \
    libsox-fmt-all \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# ==============================================================================
# Stage 2: Dependency installer (isolated layer caching)
# ==============================================================================
FROM docker.io/oven/bun:1-slim AS deps
WORKDIR /app

# Copy lockfile, package manifest, and registry configuration
COPY package.json bun.lock .npmrc ./

# Install production dependencies only using frozen lockfile
RUN bun install --frozen-lockfile --production

# ==============================================================================
# Stage 3: Production runner
# ==============================================================================
FROM base AS runner
WORKDIR /app

# Set default production environment
ENV NODE_ENV=production \
    LOG_LEVEL=info

# Copy production dependencies from deps stage
COPY --from=deps /app/node_modules ./node_modules

# Copy application configuration and source code
COPY package.json bun.lock tsconfig.json ./
COPY drizzle ./drizzle
COPY src ./src

# Ensure bot-data directory exists for session and downloads storage
RUN mkdir -p /app/bot-data /app/bot-data/downloads

# Persistent storage volume for Telegram session and database files
VOLUME ["/app/bot-data"]

# Start the bot
CMD ["bun", "run", "start"]
