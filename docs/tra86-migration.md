# tra86 → Bind migration

This document records the Phase 0 audit of the previous **tra86** implementation
and what Bind reused, rewrote, or discarded, and why.

## What tra86 was

tra86 began as an assembly-level tracing/analysis tool for comparing compiled
Rust and C++ programs. Its final state (before Bind) was a six-crate Rust
workspace with an **egui desktop GUI** and an **LLDB backend that drove the
`lldb` CLI as a subprocess and parsed its human-readable console output**:

- `tra86-core` — normalized model types
- `tra86-backend` — backend trait, errors, mock backend, thin wrapper
- `tra86-backend-lldb` — LLDB CLI adapter and text parsers
- `tra86-analysis` — register delta + x86 mnemonic classification
- `tra86-ui` — egui rendering
- `tra86-app` — app wiring, worker thread, session persistence

Its own `AUDIT.md` was refreshingly honest: it compiled and opened a window, had
the beginning of a usable LLDB adapter, but was "absolutely overstated" — the
backend was text-protocol brittle, register reads were x86-biased and behaved
badly on arm64, snapshot failures were silently swallowed, the session model was
not a real state machine, and the "capability model" was an untyped
`BTreeMap<String, bool>`.

## Why the earlier architecture was insufficient

1. **Terminal scraping as the backend API.** tra86-backend-lldb parsed the
   `lldb` CLI's human-readable output. This is fragile across LLDB versions and
   architectures and was the single largest source of correctness risk.
2. **GUI-first.** The product is a terminal tool; egui/eframe pulled in a large
   windowing/GL stack and coupled rendering to a desktop event loop.
3. **x86 assumptions throughout.** Register handling and mnemonic classification
   assumed x86-64, which broke on the arm64 host.
4. **Rust-vs-C++ framing baked in.** Language comparison was treated as close to
   the core rather than as one optional analysis.
5. **Swallowed failures / weak lifecycle.** Data-fetch failures became empty
   vectors; the session model could drift out of sync with the target.
6. **Untyped capability/event flow.** No normalized event stream; each consumer
   re-queried and re-interpreted backend state independently.

## Environment constraints discovered during the audit

The development/host machine is **arm64 macOS** with **Apple LLDB only**:

- No `lldb-dap`, `lldb-mi`, `liblldb` development libraries, or `llvm-config`,
  so the `lldb` **Rust FFI crate is not viable** here.
- The LLDB **Python SB API is available** via `PYTHONPATH="$(lldb -P)"`.
- **Live process launch/attach hangs** under LLDB in this headless context
  because current macOS requires developer-tools authorization for process
  control (see `docs/debugger-backend.md`). Static target operations (symbol
  resolution, disassembly, breakpoint resolution) work fully.
- Rust was not installed; it was added via `rustup` (stable).

These constraints drove the backend decision below.

## What Bind reused, rewrote, or discarded

| tra86 component | Disposition in Bind | Notes |
|---|---|---|
| `tra86-core` model | **Rewritten** as `bind-core` | Kept the shape of the model; added newtype IDs (`ThreadId`, `Address`, …), `AddressRange`, a real `DebugEvent`/`Command`/`Capabilities` model, arch/language isolation, and a `SessionSnapshot`. |
| `tra86-backend` trait + mock | **Rewritten** as `bind-debugger` | New `DebugBackend` trait producing normalized events; new deterministic `MockBackend` (aarch64-flavoured); added a real worker-thread orchestration layer. |
| `tra86-backend-lldb` (CLI text parsing) | **Discarded and replaced** | Replaced by `bind-lldb`, which drives the LLDB **SB API** out-of-process through an embedded Python driver over a typed JSON protocol — no console scraping. |
| `tra86-analysis` (x86 heuristics) | **Rewritten** as `bind-analysis` | Generalized `classify` to be architecture-aware; added an incremental `Analyzer` trait with confidence-bearing `Finding`s. |
| `tra86-ui` (egui) | **Discarded and replaced** | Replaced by `bind-tui` (Ratatui), rendering from immutable snapshots. |
| `tra86-app` (egui app) | **Rewritten** as `bind-cli` | clap CLI, backend selection, file logging, TUI wiring, and a noninteractive `diag` smoke driver. |
| `fixtures/native/*.cpp` | **Reused/extended** | Kept as fixtures; added `loop.c` for the LLDB integration test. |
| `AUDIT.md`, `PERF.md`, tra86 `README.md` | **Superseded** | Folded into this doc and the new `docs/`. |

## How Bind's data and control flow differ

**tra86:** UI ⇄ worker ⇄ LLDB-CLI; each pane queried the backend and parsed text
on its own; state was reconstructed ad hoc per frame.

**Bind:** one normalized event stream is the backbone.

```
key/palette ─▶ Command ─▶ [worker thread] ─▶ DebugBackend (SB API via driver)
                                   │
                                   ├─▶ SequencedEvent stream ─▶ trace ring/persist
                                   │                          └▶ analyzers ─▶ findings
                                   └─▶ SessionSnapshot ─▶ TUI renders (read-only)
```

The TUI never talks to the debugger, renders only immutable snapshots (produced
on state change, not per frame), and dispatches only typed commands. See
`docs/architecture.md`.
