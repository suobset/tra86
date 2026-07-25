# Contributing to Bind

## Ground rules

- Keep the workspace compiling and `cargo test --workspace` green.
- `cargo fmt --all` and a clean `cargo clippy --workspace --all-targets` before
  pushing.
- Respect the crate boundaries (`docs/architecture.md`): no LLDB types above
  `bind-lldb`; no register-name strings outside `bind_core::arch`; no
  demangling outside `bind-symbols`; the TUI renders snapshots and dispatches
  commands only.
- Surface failures; don't swallow them into empty vectors.
- Be honest about confidence in analyses and about capabilities in backends.

## Layout

```
crates/bind-core       domain model (no deps inward)
crates/bind-debugger   backend trait, mock, worker
crates/bind-lldb       LLDB SB-API driver + client
crates/bind-trace      ring buffer, trace format, replay
crates/bind-analysis   analyzers + findings
crates/bind-symbols    symbol index + demangling
crates/bind-storage    config/keybindings/annotations
crates/bind-tui        ratatui UI
crates/bind-cli        entry point + diag
fixtures/native        C/C++ test fixtures (compiled at test time)
docs/                  design docs
```

## Dev workflow

```bash
cargo test -p <crate>            # focused tests
cargo test --workspace          # everything
BIND_LOG=/tmp/bind.log RUST_LOG=debug cargo run -p bind-cli -- --mock   # verbose run
cargo run -p bind-cli -- diag --mock                                    # smoke report
```

Logging is off unless `BIND_LOG` names a file (the TUI owns stdout/stderr).

## Adding a backend

Implement `bind_debugger::DebugBackend` in a new crate, translate native
notifications into `bind_core::DebugEvent`s, report honest `Capabilities`, and
register it in `bind-cli`. Add integration tests that assert typed results
(not console transcripts) and skip clearly when the environment can't run them.

## Environment notes

- LLDB is required for the LLDB backend: `lldb -P` must print a python path.
- Live process control on macOS may require developer-tools authorization; tests
  handle its absence by skipping with a message, never by hanging or silently
  passing.
