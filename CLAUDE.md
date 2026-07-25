# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Bind** is a terminal-native (Ratatui) debugger, execution tracer, and runtime-analysis environment built on LLDB. It is the architectural successor to the earlier **tra86** (egui GUI + LLDB-CLI text scraping), which has been removed. Read `docs/tra86-migration.md` for the transition and `docs/architecture.md` for the big picture before making structural changes.

The repository directory is still named `tra86/` but the product is Bind; all crates are `bind-*` under `crates/`.

## Environment notes (important)

- **Rust** is installed via rustup under `~/.cargo`; a non-interactive shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo`.
- Host is **arm64 macOS with Apple LLDB only**. No `liblldb` dev libs / `llvm-config`, so the `lldb` FFI crate is not usable; Bind drives the LLDB **Python SB API** out-of-process (`lldb -P` gives the python path).
- **Live LLDB process control (launch/attach) blocks headlessly** on this macOS due to developer-tools authorization. Static SB-API ops (open target, symbols, disassembly, breakpoint resolution) work. Tests handle the live gap by skipping clearly, never hanging or faking. `timeout` is unavailable; use `perl -e 'alarm N; exec @ARGV'`.

## Commands

```bash
cargo build --release
cargo test --workspace                 # 73 tests
cargo test -p bind-core                # one crate
cargo test full_mock_debugging         # single test by substring
cargo clippy --workspace --all-targets # clean (0 warnings expected)
cargo fmt --all --check
cargo run -p bind-cli -- --mock        # run the TUI on the deterministic mock backend
cargo run -p bind-cli -- diag --mock   # noninteractive smoke report
BIND_LOG=/tmp/bind.log RUST_LOG=debug cargo run -p bind-cli -- --mock  # logging (off unless BIND_LOG set)
```

## Architecture (big picture)

Nine crates; dependency arrows point inward to `bind-core`. Nothing above `bind-lldb` references an LLDB type.

- **`bind-core`** — domain model: newtype IDs, `Address`/`AddressRange`, `arch` (the *only* place with register-name knowledge), `DebugEvent`, `Command`, `Capabilities`, `SessionSnapshot`, `BindError`.
- **`bind-debugger`** — the `DebugBackend` trait, `MockBackend` (deterministic aarch64 sim), and the worker thread. The worker owns the backend, runs commands, folds the event stream into `SessionSnapshot`s. Talks to the app via mpsc `Command`/`WorkerUpdate` channels.
- **`bind-lldb`** — LLDB backend: an embedded Python SB-API driver (`driver/bind_lldb_driver.py`, `include_str!`'d) spoken to over a JSON line protocol (`protocol.rs`) with a timeout-guarded reader thread. **Structured, not console scraping.**
- **`bind-trace`** — bounded `RingBuffer` (counts drops), versioned JSON `TraceRecord` (forwards-evolvable), reader/writer, `Replay`, filters.
- **`bind-analysis`** — incremental `Analyzer` trait → confidence-bearing `Finding`s.
- **`bind-symbols`** — symbol index + demangling (the only place with Rust/C++ mangling knowledge).
- **`bind-storage`** — JSON config/keybindings/annotations.
- **`bind-tui`** — Ratatui. `UiState` is a pure state machine; `view::render` is a pure projection of a snapshot (both tested with `TestBackend`, no terminal). Panic-safe terminal restore in `terminal.rs`.
- **`bind-cli`** — clap entry point (`bind`), backend selection, file logging, TUI wiring, and the `diag` smoke driver.

### The three shared abstractions
`SessionSnapshot` (immutable read model the TUI renders, produced on state change not per frame), `DebugEvent`/`SequencedEvent` (the one normalized stream timeline+trace+analysis all consume), and `Command` (typed intents; no raw console strings).

## Conventions the code enforces (keep these)

- Surface failures as `WorkerUpdate::CommandError` / `DebugEvent::Error`; never swallow into empty vectors.
- Architecture specifics only in `bind_core::arch`; demangling only in `bind-symbols`; LLDB only in `bind-lldb`.
- Analyses state confidence + limitations; backends report honest `Capabilities`.
- Environment-dependent tests print `SKIP: …` and return; they never silently pass.
