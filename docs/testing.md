# Testing

Bind is built so that almost everything is testable without a terminal, a
compiler, or a live debugger — the `MockBackend` and pure state machines carry
the deterministic suite, and real LLDB is exercised where the environment allows
it.

## Running

```bash
cargo test --workspace          # everything
cargo test -p bind-core         # one crate
cargo test full_mock_debugging  # one test by substring
cargo clippy --workspace --all-targets   # lint (clean)
cargo fmt --all --check         # formatting
```

## Layers

### Unit tests (no external deps)
- **bind-core:** id distinctness, address arithmetic, arch register-role
  classification, register diffs, event JSON round-trips, command round-trips.
- **bind-debugger:** the mock program (launch/step/breakpoint/step-out), and an
  end-to-end **worker** integration test (`tests/worker.rs`) driving
  launch→breakpoint→continue→step through the real channels.
- **bind-trace:** ring-buffer eviction/drop counting, filter logic, trace
  write/read round-trip, forwards-compat (unknown fields tolerated), incompatible
  major version rejected, secret-env sanitization.
- **bind-analysis:** instruction counts, function transitions, stop summaries,
  register churn, arch-aware classification.
- **bind-symbols:** nearest-symbol resolution with offsets and size bounds, C++
  and Rust demangling, cache behavior.
- **bind-storage:** default/roundtrip config, unknown-field tolerance.
- **bind-cli:** the `diag` script against the mock backend.

### TUI tests (buffer-based, no real terminal)
`bind-tui` renders into ratatui's `TestBackend` and asserts on the cell buffer:
populated render, empty state, tiny terminals (down to 1×1), ASCII fallback,
help overlay, error surfacing, timeline population, plus `UiState` transitions
(palette open/parse, focus cycling, step-key → command, Ctrl-C quit).

### Backend integration tests (real LLDB, environment-gated)
`bind-lldb/tests/lldb_integration.rs`:
- **Static ops asserted directly** against a compiled `fixtures/native/loop.c`:
  target load, symbol resolution, breakpoint resolution with source lines, and
  real instruction disassembly with bytes.
- **Full live flow** (`live_debug_full_flow_when_authorized`): launch stopped at
  entry → breakpoint on `helper` hit → non-empty live register set with a pc →
  backtrace containing `helper` called from `main` → instruction step. Asserted
  where OS debugging is authorized; **skipped with a clear message** otherwise
  (never silently passes, never hangs).

Run the live path reproducibly on Linux via the Docker harness (no macOS
authorization gate):

```bash
scripts/test-linux.sh    # cargo test -p bind-lldb with SYS_PTRACE
```

See `docs/docker.md`. Environment-dependent tests print `SKIP: …` to stderr and
return, rather than passing silently, when LLDB or a compiler is absent.

### End-to-end smoke
`bind diag [--mock]` runs a scripted launch→breakpoint→continue→step flow and
prints a report; it is both a manual smoke tool and a unit test.

## Fixtures

`fixtures/native/loop.c` (C, with a call, a loop, and `helper`) is compiled at
test time by `cc`. `fixtures/native/trace_fixture.cpp` is retained from tra86 as
an additional C++ fixture. Fixtures are compiled to the temp dir, never checked
in as binaries.
