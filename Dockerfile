# syntax=docker/dockerfile:1

# ==============================================================================
# Stage 1: Base with system dependencies (ffmpeg, sox, certificates)
# ==============================================================================
FROM docker.io/debian:bookworm-slim AS base

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
# Stage 2: Builder (stable toolchain; nightly is only needed for `just fmt`)
# ==============================================================================
FROM docker.io/rust:1-slim AS builder
WORKDIR /app

# TLS roots for crates.io / git dependencies (cargo fetches the ferogram
# git dependency over HTTPS via its bundled libgit2 — no system git needed).
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Build dependencies first for layer caching: manifests + lockfile only.
# Crate sources are stubbed (including the bot's bin target) so the
# dependency graph compiles without the real sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/engine/Cargo.toml crates/engine/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/bot/Cargo.toml crates/bot/Cargo.toml
RUN mkdir -p crates/engine/src crates/db/src crates/bot/src \
 && echo 'pub fn _stub() {}' > crates/engine/src/lib.rs \
 && echo 'pub fn _stub() {}' > crates/db/src/lib.rs \
 && echo 'pub fn _stub() {}' > crates/bot/src/lib.rs \
 && echo 'fn main() {}' > crates/bot/src/main.rs \
 && cargo build --release --locked -p bot \
 && rm -rf crates

# Real sources: build the binary (dependency layers above are reused).
COPY crates ./crates
RUN touch crates/engine/src/lib.rs crates/db/src/lib.rs crates/bot/src/lib.rs crates/bot/src/main.rs \
 && cargo build --release --locked -p bot

# ==============================================================================
# Stage 3: Production runner
# ==============================================================================
FROM base AS runner
WORKDIR /app

# Set default production environment
ENV LOG_LEVEL=info

# Copy the statically simple runtime: binary + module-free config
COPY --from=builder /app/target/release/bot ./bot

# Non-root user; bot-data holds the Telegram session and download scratch.
RUN useradd --system --home-dir /app --shell /usr/sbin/nologin alac \
 && mkdir -p /app/bot-data /app/bot-data/downloads \
 && chown -R alac:alac /app

USER alac

# Persistent storage volume for Telegram session and downloads
VOLUME ["/app/bot-data"]

# Start the bot (migrations run automatically at startup)
CMD ["./bot"]
