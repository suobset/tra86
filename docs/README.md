# Bind documentation

Start with the top-level [`../README.md`](../README.md) for what Bind is and how
to run it. These documents go deeper:

## Design
- [architecture.md](architecture.md) — crates, the three shared abstractions
  (`SessionSnapshot`, `DebugEvent`, `Command`), and the concurrency model.
- [debugger-backend.md](debugger-backend.md) — the `DebugBackend` contract, the
  LLDB SB-API driver + JSON protocol, and the live-control authorization model.
- [trace-format.md](trace-format.md) — the versioned, forwards-evolvable trace
  format, bounded capture, and replay.
- [tui.md](tui.md) — layout, keybindings, the command palette, and graceful
  degradation.

## Building, testing, shipping
- [testing.md](testing.md) — the test layers (unit, TUI buffer tests, backend
  integration, end-to-end smoke) and how to run them.
- [docker.md](docker.md) — exercising the **live** LLDB path reproducibly in a
  Linux container (`scripts/test-linux.sh`).
- [packaging-macos.md](packaging-macos.md) — Developer ID signing + notarization
  (`scripts/package-macos.sh`), entitlements, and the macOS debugging model.

## Project
- [roadmap.md](roadmap.md) — per-feature status and the next high-value steps.
- [tra86-migration.md](tra86-migration.md) — what tra86 was, why it was
  insufficient, and what Bind reused / rewrote / discarded.
- [../CONTRIBUTING.md](../CONTRIBUTING.md) — workflow and boundaries.

## Map of the repository

```
crates/            the nine bind-* crates (see architecture.md)
docker/            Dockerfile for the Linux live-LLDB test harness
docs/              this documentation
fixtures/native/   C/C++ fixtures compiled at test time
packaging/         macOS entitlements
scripts/           test-linux.sh, package-macos.sh
```
