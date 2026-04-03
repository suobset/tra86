# tra86

`tra86` is a Rust desktop assembly tracer/debugger experiment. It is still currently under development. More coming soon.

<img width="1894" height="1348" alt="image" src="https://github.com/user-attachments/assets/c3c13a46-be9f-4523-9ee7-474999857156" />

## Current Status

What is verified right now:

- `cargo check` passes
- `cargo test` passes with analysis tests, LLDB parser tests, and real LLDB integration tests against compiled C and C++ fixtures
- `cargo run -p tra86-app` launches the desktop app
- the LLDB path can launch, attach, inspect memory/registers/frames, resolve symbols/source, and step real native fixtures

What is not yet true:

- this is not a production-ready tracer/debugger
- the backend is not yet robust across common failure cases
- mid-run pause/stop is still not reliable enough on the current LLDB CLI transport, so the UI does not pretend that it is
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

<img width="1894" height="1348" alt="image" src="https://github.com/user-attachments/assets/02cc5944-9b20-4828-84e0-d10165dab086" />

## Biggest Current Problems

- the LLDB adapter is text-protocol brittle
- the session model is better than it was, but it is still not a full state machine with generation tracking
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

Interactive debugger CLI:

```bash
cargo run -p tra86-app --bin tra86_cli
cargo run -p tra86-app --bin tra86_cli -- launch /path/to/program -- arg1 arg2
cargo run -p tra86-app --bin tra86_cli -- attach 12345
```

Once the CLI starts, use `help` to list commands for launch, attach, stepping, register reads,
memory reads, breakpoints, and disassembly.

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

The next work should focus on backend reliability, a better transport for true async pause/resume behavior, typed failure handling, and a session model that does not drift out of sync when the target process does something inconvenient.
