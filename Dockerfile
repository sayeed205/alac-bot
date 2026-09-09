# syntax=docker/dockerfile:1.7

# ==============================================================================
# Stage 1: Runtime with native media support and TLS certificates
# ==============================================================================
FROM docker.io/debian:bookworm-slim AS runtime

# Native media decoding, metadata, and spectrogram rendering are provided by
# the Rust media crate. Keep only the TLS root certificates required for
# outbound HTTPS connections at runtime.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# ==============================================================================
# Stage 2: Builder
# ==============================================================================
FROM docker.io/rust:1-bookworm AS builder
WORKDIR /app

# TLS roots for crates.io / git dependencies (cargo fetches the ferogram
# git dependency over HTTPS via its bundled libgit2 — no system git needed).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates binutils \
 && rm -rf /var/lib/apt/lists/*

# Build dependencies first for layer caching: manifests + lockfile only.
# Crate sources are stubbed (including the bot's bin target) so the
# dependency graph compiles without the real sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/engine/Cargo.toml crates/engine/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/media/Cargo.toml crates/media/Cargo.toml
COPY crates/bot/Cargo.toml crates/bot/Cargo.toml
RUN --mount=type=cache,id=alac-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=alac-cargo-git,target=/usr/local/cargo/git \
    mkdir -p crates/engine/src crates/db/src crates/media/src crates/bot/src \
 && echo 'pub fn _stub() {}' > crates/engine/src/lib.rs \
 && echo 'pub fn _stub() {}' > crates/db/src/lib.rs \
 && echo 'pub fn _stub() {}' > crates/media/src/lib.rs \
 && echo 'pub fn _stub() {}' > crates/bot/src/lib.rs \
 && echo 'fn main() {}' > crates/bot/src/main.rs \
 && cargo build --release --locked -p bot \
 && rm -rf crates

# Real sources: build the binary (dependency layers above are reused).
COPY crates ./crates
RUN --mount=type=cache,id=alac-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=alac-cargo-git,target=/usr/local/cargo/git \
    touch crates/engine/src/lib.rs crates/db/src/lib.rs crates/media/src/lib.rs crates/bot/src/lib.rs crates/bot/src/main.rs \
 && cargo build --release --locked -p bot \
 && strip --strip-unneeded target/release/bot

# ==============================================================================
# Stage 3: Production runner
# ==============================================================================
FROM runtime AS runner
WORKDIR /app

# Set default production environment
ENV LOG_LEVEL=info

# Copy only the stripped runtime binary; configuration is supplied at runtime.
COPY --from=builder /app/target/release/bot ./bot

# Non-root user; bot-data holds the Telegram session and download scratch.
RUN useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin alac \
 && mkdir -p /app/bot-data/downloads \
 && chown -R alac:alac /app \
 && chmod 0755 /app/bot

USER alac

# Persistent storage volume for Telegram session and downloads
VOLUME ["/app/bot-data"]

# Start the bot (migrations run automatically at startup)
CMD ["./bot"]
