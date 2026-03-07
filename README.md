# tra86

`tra86` is a pure Rust desktop Assembly Tracer Analyzer built around a normalized execution model and pluggable backend architecture.

It is GUI-first, disassembly-centered, and designed to feel like a lightweight live execution analysis tool for macOS and Linux, with Windows support planned in the backend architecture.

## Workspace layout

- `tra86-app`: eframe/egui desktop binary, backend worker thread, session persistence
- `tra86-core`: backend-independent execution/domain model types
- `tra86-backend`: backend trait, backend-independent errors, backend orchestrator, mock backend
- `tra86-backend-lldb`: LLDB backend adapter (isolated LLDB integration)
- `tra86-analysis`: instruction delta + classification analysis helpers
- `tra86-ui`: reusable egui panels/widgets and UI event model

## Architecture

### Data model

All runtime state is represented using normalized types in `tra86-core`, including:

- `DebugSession`
- `ProcessState`, `ThreadState`, `FrameState`
- `RegisterBank`, `RegisterValue`, `RegisterDiff`
- `MemoryRegion`, `MemorySnapshot`
- `Breakpoint`, `Watchpoint`
- `InstructionRecord`, `DisassemblyLine`
- `StopReason`, `TraceEvent`, `ExecutionDelta`
- `SymbolInfo`, `SourceLocation`

This keeps UI and analysis logic backend-agnostic.

### Backend abstraction

`tra86-backend::DebugBackend` defines a stable interface for:

- launching/attaching/detaching/killing
- continue/step/pause controls
- registers, memory, threads, frames
- breakpoints
- disassembly + symbol/source lookup
- stop reason and current instruction query

`BackendOrchestrator` owns a boxed backend and enables runtime backend swapping.

### UI/worker split

`tra86-app` runs backend operations in a dedicated worker thread over channels. The egui thread only renders and dispatches intents. This prevents memory/disassembly operations from freezing the desktop UI.

### Analysis layer

`tra86-analysis` computes per-step annotations:

- changed registers
- stack pointer movement
- control-flow class (linear/jump/call/return/syscall)
- instruction hints (prologue/epilogue/branch/syscall)

These annotations are shown in instruction history rows.

## MVP capabilities

Implemented in the app shell with the mock backend and shared model:

- executable path + args input with native file picker
- launch and attach actions
- disassembly-first center pane with current-IP highlight
- step into/over/out, continue, pause, stop, restart controls
- breakpoint toggle from disassembly + right-side breakpoint list
- register pane with changed-register highlighting
- thread + frame views in left pane (stack frame context)
- memory map + hex/ascii memory inspector + numeric interpretations
- stack-oriented memory view around the selected memory address
- trace/history pane with execution deltas
- source correlation rendering when source info exists
- recent session persistence (`recent_sessions.json`)
- desktop menu bar (`File`, `Debug`, `Backend`)
- keyboard shortcuts (`F5`, `F6`, `F10`, `F11`, `Shift+F11`, `Cmd/Ctrl+R`)

## LLDB backend status

`tra86-backend-lldb` is implemented as a persistent interactive LLDB process adapter behind the same `DebugBackend` trait and isolated from other crates.

Current LLDB adapter supports:

- launch + attach
- continue / pause / step into / step over / step out
- disassembly around current instruction
- breakpoints set/remove
- register reading
- thread + frame listing
- memory read/write + memory region map
- symbol/source lookup (`image lookup`)

LLDB-specific command/process handling is still fully isolated to one crate so core, analysis, and UI layers remain debugger-agnostic.

## Why tra86 is not a debugger wrapper

- Backend output is normalized into `tra86-core` domain types before it reaches UI/analysis.
- Analysis (`ExecutionDelta`) is independent of LLDB/GDB protocol formats.
- UI speaks only `UiEvent` + normalized snapshot data, never debugger-native structures.
- Multiple backend types are planned as peers (LLDB, GDB/MI, dbgeng, instrumentation), not plugins to a single debugger UX.

## Build and run

### Prerequisites

- Rust stable toolchain (`cargo`)
- macOS or Linux desktop environment
- LLDB installed and on `PATH` for LLDB backend usage

### Commands

```bash
cargo check
cargo run -p tra86-app
```

## Screenshots

Screenshots were not captured in this headless build environment.

When running locally, recommended captures:

- `docs/screenshots/disassembly-main.png`
- `docs/screenshots/memory-trace.png`
- `docs/screenshots/register-delta.png`

## Roadmap

### Backends

- Linux-focused GDB/MI backend crate behind `DebugBackend`
- Persistent LLDB session driver (interactive process control, robust event stream)
- Windows backend crate (`dbgeng`/`cdb`-style adapter) using the same normalized model

### Architecture and ISA

- ISA modules beyond x86_64 (AArch64 first)
- stronger symbol/source integration and inlined frame handling
- backend capability negotiation with finer feature flags

### Trace and analysis

- full trace recording mode and trace export/import
- trace replay/time navigation groundwork
- memory write tracking with richer deltas
- basic block/function heatmaps
- watch expressions and dataflow-oriented views

## Platform assumptions

- First-class target: macOS + Linux
- Windows support is intentionally designed into traits/models but not yet fully implemented
- Pure Rust stack only (no Electron/Tauri/Java/web frontend)
