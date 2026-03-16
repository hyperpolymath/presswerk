# SPDX-License-Identifier: PMPL-1.0-or-later
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# Presswerk — Verified container build (stapeln scheme)
#
# Follows the hyperpolymath container ecosystem standard:
#   Base:    cgr.dev/chainguard (wolfi-base build, static runtime)
#   Runtime: vordr (formally verified container execution)
#   Bridge:  selur (zero-copy IPC)
#   Gateway: svalinn (HTTP policy enforcement for IPP)
#   Signing: cerro-torre (Ed25519 signatures, .ctp bundles)
#   Secrets: rokur (secrets management)
#
# Build:   podman build -f Containerfile -t presswerk:latest .
# Run:     podman run --rm -p 631:631 -v presswerk-data:/var/lib/presswerk presswerk:latest
# Seal:    selur seal presswerk:latest
# Sign:    cerro-torre sign presswerk:latest
# Verify:  cerro-torre verify presswerk:latest

# ============================================================
# Stage 1: Build Presswerk (Rust)
# ============================================================
FROM cgr.dev/chainguard/wolfi-base:latest AS presswerk-builder

RUN apk add --no-cache \
    rust \
    cargo \
    gcc \
    glibc-dev \
    pkgconf \
    openssl-dev

WORKDIR /build/presswerk
COPY . .

# Build the headless print server (no desktop UI deps needed)
# SQLite is bundled via rusqlite, no system libsqlite required
RUN cargo build --release \
    -p presswerk-core \
    -p presswerk-security \
    -p presswerk-document \
    -p presswerk-print

# ============================================================
# Stage 2: Build Selur WASM Bridge (optional — for vordr IPC)
# ============================================================
FROM cgr.dev/chainguard/wolfi-base:latest AS selur-builder

RUN apk add --no-cache rust cargo

WORKDIR /build/selur
# Selur bridge is optional — build if present, skip otherwise
COPY selur/ /build/selur/ 2>/dev/null || true
RUN if [ -f Cargo.toml ]; then \
      rustup target add wasm32-wasi 2>/dev/null || true && \
      cargo build --release --target wasm32-wasi 2>/dev/null || true; \
    fi

# ============================================================
# Stage 3: Assemble Verified Runtime Image
# ============================================================
FROM cgr.dev/chainguard/wolfi-base:latest AS runtime

# Minimal runtime dependencies
RUN apk add --no-cache \
    glibc \
    libgcc \
    openssl \
    ca-certificates

# Non-root user (security hardening)
RUN adduser -D -h /app -s /sbin/nologin presswerk
USER presswerk
WORKDIR /app

# Copy Presswerk binaries from builder
COPY --from=presswerk-builder --chown=presswerk:presswerk \
    /build/presswerk/target/release/libpresswerk_print.so ./lib/ 2>/dev/null || true
COPY --from=presswerk-builder --chown=presswerk:presswerk \
    /build/presswerk/target/release/libpresswerk_security.so ./lib/ 2>/dev/null || true
COPY --from=presswerk-builder --chown=presswerk:presswerk \
    /build/presswerk/target/release/libpresswerk_document.so ./lib/ 2>/dev/null || true

# Copy Selur WASM bridge (if built)
COPY --from=selur-builder --chown=presswerk:presswerk \
    /build/selur/target/wasm32-wasi/release/*.wasm ./lib/ 2>/dev/null || true

# Create directory structure (stapeln pattern)
RUN mkdir -p \
    /var/lib/presswerk \
    /var/lib/presswerk/documents \
    /var/lib/presswerk/jobs.db \
    /var/lib/presswerk/audit.db \
    /etc/presswerk \
    /var/cache/presswerk

# ── Configuration ──────────────────────────────────────────────────────────

# Vordr integration (formally verified container execution)
ENV VORDR_ENDPOINT=selur://unix:///run/presswerk.sock

# Selur bridge (zero-copy IPC)
ENV SELUR_WASM=/app/lib/presswerk-bridge.wasm

# Svalinn policy enforcement (HTTP gateway for IPP)
ENV SVALINN_PORT=8000
ENV SVALINN_POLICY=/etc/presswerk/svalinn-policy.yaml

# Presswerk configuration
ENV PRESSWERK_PORT=631
ENV PRESSWERK_DATA_DIR=/var/lib/presswerk
ENV PRESSWERK_LOG_LEVEL=info
ENV PRESSWERK_HEADLESS=true

# Cerro-Torre trust store (Ed25519 signing)
ENV CT_TRUST_STORE=/etc/presswerk/trust-store

# ── Ports ──────────────────────────────────────────────────────────────────

# IPP print server
EXPOSE 631

# Svalinn HTTP gateway (if running svalinn in front)
EXPOSE 8000

# ── Health Check ───────────────────────────────────────────────────────────

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q --spider http://localhost:${PRESSWERK_PORT}/ipp/print || exit 1

# ── Entry Point ────────────────────────────────────────────────────────────

ENTRYPOINT ["/app/presswerk"]
CMD ["--headless", "--port", "631", "--data-dir", "/var/lib/presswerk"]

# ── OCI Labels ─────────────────────────────────────────────────────────────

LABEL org.opencontainers.image.title="Presswerk"
LABEL org.opencontainers.image.description="High-assurance local print router/server — IPP/1.1 server, mDNS discovery, document scanning, encrypted storage, formal verification"
LABEL org.opencontainers.image.url="https://github.com/hyperpolymath/presswerk"
LABEL org.opencontainers.image.source="https://github.com/hyperpolymath/presswerk"
LABEL org.opencontainers.image.version="0.3.0"
LABEL org.opencontainers.image.licenses="PMPL-1.0-or-later"
LABEL org.opencontainers.image.authors="Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>"
LABEL org.opencontainers.image.vendor="hyperpolymath"
LABEL org.opencontainers.image.base.name="cgr.dev/chainguard/wolfi-base:latest"
LABEL dev.hyperpolymath.container.runtime="vordr"
LABEL dev.hyperpolymath.container.bridge="selur"
LABEL dev.hyperpolymath.container.gateway="svalinn"
LABEL dev.hyperpolymath.container.signing="cerro-torre"
LABEL dev.hyperpolymath.container.scheme="stapeln"
