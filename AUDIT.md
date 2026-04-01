# tra86 Audit

Date: 2026-03-31

This repo is not a wreck, but it is absolutely overstated. It compiles, it opens a GUI window, and it has the beginning of a usable LLDB adapter. It is not yet a reliable tracer/debugger.

## Verified Current State

- `cargo check`: passes after installing a Rust toolchain on this machine
- `cargo test`: passes, but only because there are effectively no tests
- `cargo run -p tra86-app`: failed before this audit because the package exposed two binaries and no `default-run`
- `cargo run -p tra86-app --bin tra86-app`: launches the desktop app
- `cargo run -p tra86-app --bin lldb_smoke -- /private/tmp/tra86-fixture/sample`: runs, returns disassembly, but only found a single register on arm64 during this audit

## What Is Real vs Aspirational

### Real

- `tra86-app`: real eframe/egui shell with a backend worker thread
- `tra86-backend-lldb`: real LLDB process adapter, but text-protocol brittle
- `tra86-backend`: real trait surface and a thin backend wrapper
- `tra86-core`: real normalized model types, although broader than the code actually uses
- `tra86-ui`: real rendering code, but mostly a monolithic panel module rather than a reusable UI library

### Aspirational or overstated

- “Meaningful tests”: false today
- “Reliable tracing MVP”: false today
- “Backend-agnostic architecture”: only partially true; the type split exists, but the real behavior is almost entirely LLDB-specific
- “Trace analysis layer”: technically present, but currently a handful of mnemonic heuristics
- “Capability model”: not a real model, just `BTreeMap<String, bool>`

## Dead Code, Redundant Abstractions, Fake Architecture

- `BackendOrchestrator` adds almost no value beyond owning `Box<dyn DebugBackend>`.
- `DebugSession` is not driving the app. It looks central in the README and irrelevant in actual runtime flow.
- `Watchpoint` exists in the core model but has no real pipeline behind it.
- `capabilities()` returning `BTreeMap<String, bool>` is architecture theater, not a durable interface.
- The mock backend is useful for UI bring-up, but it currently makes the app look more complete than the real backend path is.
- `egui_dock` is declared but not used.

## Correctness Risks

- LLDB register reads are architecture-fragile. The current explicit register request is x86-heavy and behaved badly on arm64 during this audit.
- Launch currently stops in loader/runtime code rather than proving a good user-facing stop point.
- Backend snapshot collection silently converts many backend failures into empty vectors. That keeps the UI alive but hides real faults and desynchronization.
- Trace deduplication is keyed only by instruction address, which can collapse repeated visits to the same IP into misleading history.
- Argument parsing is `split_whitespace()`, which breaks quoting and escaped arguments.
- The UI can call `std::process::exit(0)` directly, which is an ugly shutdown path for a stateful tool.

## Performance Risks

- Snapshot refreshes fetch a lot of data eagerly and repeatedly.
- Disassembly fetches are large by default and not obviously bounded by user need.
- Trace history is cloned into UI state on every update.
- There is no cache invalidation discipline for disassembly, memory, or symbols.
- There is no profiling or measurement in the repo today.

## Concurrency and UI-Thread Risks

- The dedicated worker thread is the right direction.
- There is no formal session state machine, so process exit/detach/error paths can leave stale UI state behind.
- Commands are serialized, but there is no cancellation or generation tracking. A slow backend reply can still arrive after the user has changed direction.
- Error handling is inconsistent: command failures sometimes surface, data-fetch failures often disappear.

## Backend Design Risks

- The backend trait mixes control, querying, symbol lookup, memory, and disassembly with no typed capability discipline.
- LLDB parsing is line-oriented and heuristic-heavy. It will break on output variation.
- The adapter logs almost nothing structured about command/response flow, which makes backend bugs hard to diagnose.
- Backend-native assumptions still leak upward in practice through incomplete normalization and fallback behavior.

## Where The Design Is Overengineered

- Six crates for a codebase this small is only justified if the boundaries stay honest. Right now several boundaries are mostly conceptual.
- The capability map is more abstraction than substance.
- The README describes a larger product than the implementation currently supports.

## Where The Design Is Underdesigned

- No real session lifecycle/state machine
- No typed backend capability or failure taxonomy beyond generic backend errors
- No integration tests against real debuggable binaries
- No bounded trace/history policy beyond ad hoc truncation
- No structured backend logging
- No performance measurement framework

## Shortest Path To A Real MVP

1. Stop lying in docs and packaging.
2. Add real LLDB smoke/integration tests against a tiny native fixture binary.
3. Fix LLDB register/disassembly/frame parsing on current target architectures.
4. Stop swallowing backend refresh failures; surface them explicitly in UI state and logs.
5. Add a minimal session lifecycle model so exit/detach/crash do not leave stale data onscreen.
6. Tighten trace/history handling so repeated stepping does not generate misleading state.
7. Only after the backend is trustworthy, spend time on layout and UX refinement.

## Prioritized Plan

### P0: must-fix blockers

- Make `cargo run -p tra86-app` work without extra binary flags
- Add meaningful tests; zero-test green builds are not useful
- Harden LLDB register parsing on arm64 and x86_64
- Replace silent snapshot failure paths with diagnosable errors
- Verify launch, step, disassembly, registers, frames, memory, and breakpoints on a real fixture binary

### P1: MVP-critical work

- Introduce explicit session lifecycle handling in the app worker
- Improve launch/attach behavior so the initial stop is useful
- Add robust error display in the GUI
- Improve argument handling and path validation
- Add structured backend command/response logging
- Bound refresh work and reduce obviously redundant backend calls

### P2: post-MVP improvements

- Cache disassembly/symbol/memory data with clear invalidation rules
- Improve register diffing and trace model quality
- Add attach-focused tests and exit/crash-path tests
- Remove or quarantine abstractions that still overpromise capability

### P3: nice-to-haves

- Stronger source correlation
- Better keyboard and navigation UX
- Better visual treatment for large traces and memory views
- Future backend prep once the LLDB path is actually dependable
