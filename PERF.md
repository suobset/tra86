# tra86 Performance Notes

Date: 2026-03-31

This file is intentionally factual. It is not a victory lap. The current work has only started performance measurement, and almost all meaningful optimization still needs to happen after the tracing path is more complete.

## What Was Measured

Environment:

- macOS arm64
- local debug fixture binary compiled with `cc -g -O0`
- command under test: `target/debug/lldb_smoke /private/tmp/tra86-fixture/sample`

The smoke binary currently does:

- open target
- disassemble pre-launch
- launch target
- list threads
- read registers
- disassemble again
- step once
- disassemble again

## Current Hotspots

- LLDB process startup and command round-trips dominate the smoke path
- backend work is still very chatty and mostly uncached
- full register dumps and large text parsing are still heavier than they need to be
- app refresh still asks for broad snapshots rather than minimal deltas

## Measured Change

### Before teardown fix

- one measured run: `real 25.20s`

Observed cause:

- backend teardown was waiting on a `quit` command path that could stall badly during drop

### After teardown fix

- measured run: `real 5.38s`
- `user 0.44s`
- `sys 0.11s`

What changed:

- LLDB process startup now prefers direct stdio instead of forcing the PTY wrapper first
- LLDB teardown no longer blocks on the old `quit` round-trip during drop
- parser fixes also reduced false-empty results that caused retries and fallback behavior

## What Improved

- smoke-test end-to-end runtime improved from roughly `25.20s` to `5.38s`
- the LLDB smoke integration test now completes in about `6.17s` during `cargo test`
- register and memory fetches now return real data instead of silently degrading to partial output

## What Still Needs Work

- cache disassembly and symbol lookups across refreshes
- avoid fetching full program disassembly on every snapshot when the UI only needs a window
- reduce clone-heavy trace propagation into UI state
- add profiling around stepping loops and repeated refreshes inside the GUI, not just the smoke binary
- measure app responsiveness under sustained stepping instead of only backend CLI smoke

## Claims I Am Not Making

- I am not claiming the tracer is fast overall yet
- I am not claiming the GUI refresh path is optimized
- I am not claiming the backend call pattern is efficient

The current measured win is real, but it is a correctness-adjacent fix that also removed a very obvious latency bug. The real performance phase still needs deliberate profiling and narrower refresh logic.
