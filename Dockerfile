# ── Build stage ──────────────────────────────────────────────────────────
ARG RUST_VERSION=1.96
FROM rust:${RUST_VERSION}-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && \
    apt-get install --no-install-recommends -y ca-certificates pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from source changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --locked --features full --bin pano && \
    rm -rf src

# Build the real binary.
COPY src/ src/
RUN touch src/main.rs && cargo build --release --locked --features full --bin pano

# ── Runtime stage ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Install only what the binary actually needs at runtime:
#   ca-certificates → HTTPS RPC and webhook calls via reqwest/aws-lc-rs.
RUN apt-get update && \
    apt-get install --no-install-recommends -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for the process.
RUN useradd --create-home --shell /bin/bash --uid 1000 pano

# Default config directory.
RUN mkdir -p /etc/pano && chown pano:pano /etc/pano

COPY --from=builder /app/target/release/pano /usr/local/bin/pano

LABEL org.opencontainers.image.title="Pano" \
      org.opencontainers.image.description="Multi-chain deposit detector" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.source="https://github.com/melonask/pano"

# Default config path; override via --config, PANO_CONFIG env, or mount.
ENV PANO_CONFIG=/etc/pano/Config.toml

EXPOSE 3210
USER pano
HEALTHCHECK --interval=30s --timeout=3s --retries=3 CMD ["pano", "ping"]
ENTRYPOINT ["pano"]
