# Bind

**Bind** is a terminal-native debugger, execution tracer, and runtime-analysis
environment built on LLDB. It sits between a raw debugger and a program-analysis
tool: more structured than driving `lldb` by hand, more interactive than an
offline trace dump, and language- and architecture-independent by design.

Bind is the architectural successor to **tra86** (see
[`docs/tra86-migration.md`](docs/tra86-migration.md)). It is an early but
coherent, fully-tested core — not yet a finished product.

## What Bind is

- A **TUI-first** debugger (Ratatui): source/disassembly, registers with change
  highlighting, stack/frames, memory, a typed event **timeline**, and live
  **analysis** findings.
- Built on a **typed event model** — every backend notification becomes a
  normalized `DebugEvent`; the UI, trace, and analyses all consume the same
  stream.
- **Backend-independent**: LLDB is one implementation behind a trait; a
  deterministic mock backend runs the entire app with no debugger.
- **Architecture- and language-independent** in the model: aarch64 and x86-64
  register handling is isolated; source language is optional metadata.

## What Bind is not (yet)

- Not a finished, production debugger.
- Live LLDB process control is gated by OS authorization on macOS (see below);
  static analysis of a target works regardless.
- No variable evaluation view, watchpoints, or core-file open yet (roadmapped).

## Maturity

See [`docs/roadmap.md`](docs/roadmap.md) for a per-feature status table. The
core (model, backend trait, mock, worker, TUI, trace, analysis, symbols) is
implemented and tested. The LLDB backend's **static** operations are covered by
a real integration test, and the **full live flow** (launch → breakpoint →
registers → backtrace → step) is asserted end to end via the Docker harness
([`docs/docker.md`](docs/docker.md)) and on authorized macOS.

## Supported hosts & targets

- **Hosts:** macOS (arm64 validated) and Linux. LLDB must be installed.
- **Targets:** any binary LLDB can inspect — C, C++, Rust, Swift, Objective-C,
  Zig, other LLVM languages, mixed-language and stripped binaries (with reduced
  capability). C and Rust/C++ are the primary validation targets.

## LLDB requirement

Bind drives the LLDB **Python SB API** out-of-process (it does not link
liblldb). It needs `lldb` on `PATH` such that `lldb -P` prints the LLDB python
path. On macOS this ships with Xcode / Command Line Tools; on Debian/Ubuntu
`apt install lldb`, on Fedora `dnf install lldb python3-lldb`.

## Install & build

```bash
# Rust stable toolchain required (https://rustup.rs)
cargo build --release
cargo test --workspace
```

**Live debugging authorization.** Static inspection needs nothing special. Live
launch/attach requires OS debugging authorization:

- **macOS:** `sudo DevToolsSecurity -enable` (once). Without it the first attach
  blocks on a system prompt; Bind times out into a clear error rather than
  hanging.
- **Linux:** just the `SYS_PTRACE` capability. Run the reproducible live-path
  harness with `scripts/test-linux.sh` (see [`docs/docker.md`](docs/docker.md)).

**Distributable macOS build.** `scripts/package-macos.sh` produces a universal,
Developer ID-signed, notarized, stapled `.pkg` — see
[`docs/packaging-macos.md`](docs/packaging-macos.md).

## Usage

```bash
bind ./program                 # launch under the LLDB backend
bind ./program -- arg1 arg2    # pass target arguments
bind attach 12345              # attach to a pid
bind open core.dump ./program  # open a core file (scaffolded)
bind --mock                    # run the deterministic mock backend (no LLDB)
bind diag --mock               # noninteractive smoke report
```

If LLDB isn't detected, Bind falls back to the mock backend with a warning.

### Keys

`c` continue · `n` next · `s` step-in · `o` finish · `i` instruction ·
`Tab` focus · `:` command palette · `/` search · `?` help · `q` / `Ctrl-C` quit.
Full command grammar in [`docs/tui.md`](docs/tui.md).

## Known limitations

- **macOS live debugging** requires developer-tools authorization
  (`sudo DevToolsSecurity -enable`); without it the first attach blocks and Bind
  times out gracefully. See [`docs/debugger-backend.md`](docs/debugger-backend.md).
- No variable/locals evaluation view, watchpoints, or core-file open yet
  (roadmapped).
- The analysis and trace layers are wired into the TUI but still shallow
  compared to the eventual goal (see the roadmap).

## Documentation

Full index: [`docs/README.md`](docs/README.md).

- [Architecture](docs/architecture.md)
- [Debugger backend](docs/debugger-backend.md)
- [Trace format](docs/trace-format.md)
- [TUI](docs/tui.md)
- [Testing](docs/testing.md)
- [Live LLDB via Docker](docs/docker.md)
- [macOS packaging & notarization](docs/packaging-macos.md)
- [Roadmap](docs/roadmap.md)
- [tra86 → Bind migration](docs/tra86-migration.md)
- [Contributing](CONTRIBUTING.md)

## License

MIT.
