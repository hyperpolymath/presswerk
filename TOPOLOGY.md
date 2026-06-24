<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- TOPOLOGY.md — Project architecture map and completion dashboard -->
<!-- Last updated: 2026-04-04 -->

# Presswerk — Project Topology

## System Architecture

```
                        ┌─────────────────────────────────────────┐
                        │              MOBILE USER                │
                        │        (iOS / Android / Desktop)        │
                        └───────────────────┬─────────────────────┘
                                            │
                                            ▼
                        ┌─────────────────────────────────────────┐
                        │           DIOXUS APP LAYER              │
                        │    (Pure Rust UI, 10 Pages, State)      │
                        └──────────┬───────────────────┬──────────┘
                                   │                   │
                                   ▼                   ▼
                        ┌───────────────────────┐  ┌────────────────────────────────┐
                        │ CORE SERVICES (RUST)  │  │ SECURITY & VERIF               │
                        │ - Print (IPP/LPR)     │  │ - Idris2 ABI (Proofs)          │
                        │ - Document (PDF/OCR)  │  │ - Encrypted Storage (age)      │
                        │ - Bridge (iOS/Android)│  │ - Audit Trail (SQLite)         │
                        └──────────┬────────────┘  └──────────┬─────────────────────┘
                                   │                          │
                                   └────────────┬─────────────┘
                                                ▼
                        ┌─────────────────────────────────────────┐
                        │           INTERFACE LAYER (FFI)         │
                        │  ┌───────────┐  ┌───────────────────┐  │
                        │  │  Zig FFI  │  │  C ABI Bridge     │  │
                        │  │  (System) │  │  (Shared Libs)    │  │
                        │  └─────┬─────┘  └────────┬──────────┘  │
                        └────────│─────────────────│──────────────┘
                                 │                 │
                                 ▼                 ▼
                        ┌─────────────────────────────────────────┐
                        │           NETWORK PRINTERS              │
                        │      (mDNS, SNMP, IPP, LPR, TCP)        │
                        └─────────────────────────────────────────┘

                        ┌─────────────────────────────────────────┐
                        │          REPO INFRASTRUCTURE            │
                        │  Justfile Automation  .machine_readable/  │
                        │  Cargo Workspace      0-AI-MANIFEST.a2ml  │
                        │  Groove Protocol      .well-known/groove/ │
                        └─────────────────────────────────────────┘
```

## Completion Dashboard

```
COMPONENT                          STATUS              NOTES
─────────────────────────────────  ──────────────────  ─────────────────────────────────
CORE APP (DIOXUS)
  Dioxus UI (presswerk-app)         ██████████ 100%    10 pages, mobile-native stable
  Print Engine (presswerk-print)    ██████████ 100%    RFC 8010/8011 client active
  Document Ops (presswerk-doc)      ██████████ 100%    PDF/Image/OCR stable
  Bridge (iOS objc2 / Android JNI)  ██████████ 100%    Native FFI verified

SECURITY & PROOFS
  Idris2 ABI (5 Proof Files)        ██████████ 100%    0 Admitted, 0 believe_me
  Encrypted Storage (age)           ██████████ 100%    X25519 at rest stable
  Audit Trail (Append-only)         ██████████ 100%    Encrypted SQLite verified
  Zig FFI Bridge                    ██████████ 100%    C-compatible implementation

REPO INFRASTRUCTURE
  Justfile Automation               ██████████ 100%    Standard build/verify tasks
  .machine_readable/                ██████████ 100%    STATE tracking active
  Test Suite (68 passing)           ██████████ 100%    High core logic coverage

─────────────────────────────────────────────────────────────────────────────
OVERALL:                            ██████████ 100%    v1.0.0 Production Ready
```

## Key Dependencies

```
Idris2 ABI ──────► Zig FFI Bridge ──────► Rust Bridge ──────► Native UI
     │                 │                    │                   │
     ▼                 ▼                    ▼                   ▼
Protocol Proof ──► Print Engine ───────► Discovery ────────► Network
```

## Update Protocol

This file is maintained by both humans and AI agents. When updating:

1. **After completing a component**: Change its bar and percentage
2. **After adding a component**: Add a new row in the appropriate section
3. **After architectural changes**: Update the ASCII diagram
4. **Date**: Update the `Last updated` comment at the top of this file

Progress bars use: `█` (filled) and `░` (empty), 10 characters wide.
Percentages: 0%, 10%, 20%, ... 100% (in 10% increments).
