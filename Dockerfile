# Multi-stage build for the Paschal Beacon.
#
# Stage 1 (frontend):     build the SvelteKit app → apps/principal/build/
# Stage 2 (chef-planner): produce a Rust dependency recipe for layer caching
# Stage 3 (chef-builder): cook deps, then build the Rust binary (which embeds
#                         the frontend at compile time via include_dir!)
# Stage 4 (runtime):      small Debian-slim image with the binary + assets

# ---- Frontend ---------------------------------------------------------------
FROM node:22-slim AS frontend
WORKDIR /build/apps/principal
ENV CI=true
RUN npm install -g pnpm@10 --silent
COPY apps/principal/ ./
# Wipe any node_modules that slipped past .dockerignore, then install + build.
RUN rm -rf node_modules .svelte-kit build \
 && pnpm install --frozen-lockfile \
 && pnpm build

# ---- Rust planner -----------------------------------------------------------
FROM rust:1.95-slim-bookworm AS chef-planner
WORKDIR /build
RUN cargo install cargo-chef --locked --version 0.1.71
# We don't need the frontend to compute the recipe.
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- Rust builder -----------------------------------------------------------
FROM rust:1.95-slim-bookworm AS chef-builder
WORKDIR /build
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked --version 0.1.71

# Cook the dependency layer.
COPY --from=chef-planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --bin beacon

# Copy everything else, plus the frontend build from stage 1.
COPY . .
COPY --from=frontend /build/apps/principal/build apps/principal/build

# Build the binary (beacon embeds apps/principal/build/ at compile time).
RUN cargo build --release --bin beacon

# ---- Runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN useradd --create-home --uid 10001 --shell /usr/sbin/nologin paschal

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 wget \
 && rm -rf /var/lib/apt/lists/*

COPY --from=chef-builder /build/target/release/beacon /app/beacon
COPY migrations /app/migrations
COPY apps/heir /app/apps/heir

RUN chown -R paschal:paschal /app && \
    mkdir -p /var/lib/paschal/blobs /var/lib/paschal/logs && \
    chown -R paschal:paschal /var/lib/paschal

USER paschal

ENV BEACON_BIND=0.0.0.0:8080 \
    BEACON_BLOB_ROOT=/var/lib/paschal/blobs \
    BEACON_KMS_KEY_PATH=/var/lib/paschal/kms.key \
    STUB_NOTIFICATIONS_LOG=/var/lib/paschal/logs/notifications.log \
    BEACON_LOG_FORMAT=json \
    RUST_LOG=info

EXPOSE 8080

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD ["sh", "-c", "wget -q --spider http://127.0.0.1:8080/livez || exit 1"]

ENTRYPOINT ["/app/beacon"]
