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
| LLDB backend — live control (launch/step/registers/frames) | 🟡 (blocked by macOS authorization here; works where debugging is authorized) |
| `diag` noninteractive smoke | ✅ |
| Watchpoints | 🟦 (modeled, not wired) |
| Core-file open | 🟦 (CLI + spec present, driver op deferred) |
| Trace persistence wired into the live TUI loop | 🟡 (recorder + commands exist; app-loop wiring is minimal) |
| JIT event representation | 🟦 (event/module fields present; no loader integration) |

## Next five highest-value steps

1. **Wire the trace recorder into the app loop.** The worker already emits the
   sequenced stream; connect `TraceRecorder` in `bind-cli::run_tui` so
   `trace start/stop` persists and the timeline shows real drop counts.
2. **Live LLDB on Linux CI.** Add a Linux CI job where LLDB process control is
   unauthenticated, and promote the live integration test from skip to asserted
   there (launch → breakpoint → step → registers).
3. **Frame/variable evaluation.** Add driver ops for locals/arguments with
   per-variable error isolation, and a variables view.
4. **Watchpoints end to end.** Driver op + backend method + UI, with honest
   capability reporting per target.
5. **Register groups & wide registers.** Surface vector/flag groups and
   `>64-bit` registers (the `Register::wide` field exists) with x86-64 coverage
   alongside aarch64.

## Deliberately out of scope (for now)

Cloud/collaboration, plugin marketplace, desktop GUI, AI debugging advice, full
record-and-replay, kernel/remote debugging, universal JIT, replacing LLDB.
