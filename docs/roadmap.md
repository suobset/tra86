# Roadmap

Bind's first slice prioritizes a trustworthy local debugging/tracing core over
breadth. Status legend: ✅ done & tested · 🟡 implemented, not integration-tested
· 🟦 scaffolded/designed · ⬜ deferred.

## Current status

| Area | Status |
|---|---|
| Domain model, events, commands, capabilities | ✅ |
| Backend trait + mock backend + worker thread | ✅ |
| TUI shell (layout, panels, palette, help, panic-safe restore) | ✅ |
| Trace ring buffer + versioned format + replay | ✅ |
| Analyses (stop summary, instruction counts, transitions, register churn) | ✅ |
| Symbols (index + demangling + cache) | ✅ |
| Config/keybindings/annotations | ✅ |
| LLDB backend — static ops (target/symbols/disasm/breakpoints) | ✅ |
| LLDB backend — live control (launch/breakpoint/registers/backtrace/step) | ✅ (asserted in the Docker harness and on authorized macOS) |
| Docker harness for live LLDB (`scripts/test-linux.sh`) | ✅ |
| macOS packaging: sign + notarize (`scripts/package-macos.sh`) | ✅ scripted; needs your Developer ID to run |
| `diag` noninteractive smoke | ✅ |
| Watchpoints | 🟦 (modeled, not wired) |
| Core-file open | 🟦 (CLI + spec present, driver op deferred) |
| Trace persistence wired into the live TUI loop | 🟡 (recorder + commands exist; app-loop wiring is minimal) |
| JIT event representation | 🟦 (event/module fields present; no loader integration) |

## Next five highest-value steps

1. **CI wiring.** Turn `scripts/test-linux.sh` (live LLDB) and the host
   `cargo test`/`clippy`/`fmt` into a GitHub Actions workflow so the live path
   runs on every push.
2. **Frame/variable evaluation.** Add driver ops for locals/arguments with
   per-variable error isolation, and a variables view.
3. **Watchpoints end to end.** Driver op + backend method + UI, with honest
   capability reporting per target.
4. **Register groups & wide registers.** Surface vector/flag groups and
   `>64-bit` registers (the `Register::wide` field exists) with x86-64 coverage
   alongside aarch64.
5. **Homebrew tap / release automation.** Wrap the notarized `.pkg` from
   `scripts/package-macos.sh` in a tap and cut tagged releases.

## Deliberately out of scope (for now)

Cloud/collaboration, plugin marketplace, desktop GUI, AI debugging advice, full
record-and-replay, kernel/remote debugging, universal JIT, replacing LLDB.
