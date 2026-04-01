# tra86

`tra86` is a Rust desktop assembly tracer/debugger experiment. The direction is serious. The current implementation is not yet serious enough.

As of 2026-03-31, this repo compiles, the app launches, and the LLDB path can drive a real target, but reliability is still below the bar for a systems tool. The most honest status report is [`AUDIT.md`](./AUDIT.md).

## Current Status

What is verified right now:

- `cargo check` passes
- `cargo test` passes with analysis tests, LLDB parser tests, and a real LLDB smoke test against a compiled native fixture
- `cargo run -p tra86-app` launches the desktop app
- the LLDB smoke binary can launch, inspect, read memory from, and step through a real fixture program

What is not yet true:

- this is not a production-ready tracer/debugger
- the backend is not yet robust across common failure cases
- the analysis layer is still thin
- the architecture still contains a few abstractions that overpromise capability

## Workspace

- `tra86-core`: normalized execution/domain types
- `tra86-backend`: backend trait, errors, mock backend, thin backend wrapper
- `tra86-backend-lldb`: LLDB-specific adapter and parsers
- `tra86-analysis`: register delta and instruction classification helpers
- `tra86-ui`: egui rendering and UI event model
- `tra86-app`: application wiring, worker thread, session persistence

## What Already Exists

- egui desktop shell
- backend worker thread to keep debugger work off the UI thread
- disassembly, registers, frames, threads, breakpoints, memory, trace panes
- LLDB adapter behind a Rust trait instead of embedding LLDB types in the UI
- a mock backend that is useful for UI work but currently makes the product look farther along than it is

## Biggest Current Problems

- the LLDB adapter is text-protocol brittle
- there is still no explicit session state machine
- some backend failure paths are now surfaced in the UI, but lifecycle recovery is still incomplete
- test coverage is now real but still thin compared to the size of the product goal
- some abstractions are more decorative than proven

## Build And Run

Prerequisites:

- Rust stable toolchain
- LLDB installed and available on `PATH`
- macOS or Linux desktop environment

Commands:

```bash
cargo check
cargo test
cargo run -p tra86-app
```

Useful smoke check:

```bash
cargo run -p tra86-app --bin lldb_smoke -- /path/to/debuggable/binary
```

Performance notes and current hotspots live in [`PERF.md`](./PERF.md).

## Direction

The intended product is still the same:

- Rust-native desktop application
- backend-agnostic normalized execution model
- real tracing/debugging on native binaries
- strong debugger fidelity before UI polish

The next work should focus on backend reliability, integration tests, typed failure handling, and a session model that does not drift out of sync when the target process does something inconvenient.
