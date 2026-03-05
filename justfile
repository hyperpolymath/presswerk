# SPDX-License-Identifier: PMPL-1.0-or-later
# Presswerk — Build recipes

# Default: run desktop dev server
default: dev

# Run desktop dev server
dev:
    dx serve --platform desktop

# Run on iOS simulator
ios:
    dx serve --platform ios

# Run on Android emulator
android:
    dx serve --platform android

# Build release
build:
    cargo build --release --workspace

# Run all tests
test:
    cargo test --workspace

# Run library tests only (avoids desktop linker deps)
test-libs:
    cargo test -p presswerk-core -p presswerk-security -p presswerk-document -p presswerk-print

# Run all benchmarks
bench:
    cargo bench --workspace

# Clippy lint
lint:
    cargo clippy --workspace -- -D warnings

# Format check
fmt-check:
    cargo fmt --all -- --check

# Format fix
fmt:
    cargo fmt --all

# Security scan
assail:
    panic-attack assail .

# Type-check all Idris2 ABI proofs
verify-abi:
    cd src/abi && idris2 --check Types.idr && idris2 --check Protocol.idr && idris2 --check Encryption.idr && idris2 --check Layout.idr && idris2 --check Bridge.idr

# Full CI check (test + lint + fmt)
ci: test-libs lint fmt-check

# Clean build artifacts
clean:
    cargo clean

# [AUTO-GENERATED] Multi-arch / RISC-V target
build-riscv:
	@echo "Building for RISC-V..."
	cross build --target riscv64gc-unknown-linux-gnu
