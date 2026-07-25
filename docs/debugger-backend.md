# The debugger backend

## The contract

`bind_debugger::DebugBackend` is the seam between Bind and any debugging engine.
It is synchronous (the worker thread pumps it) and produces normalized events
via `poll_events`. It names no LLDB type. `MockBackend` and `LldbBackend` both
implement it; the core, TUI, trace, and analysis layers run identically against
either — which is what makes the whole app testable without a live debugger.

Capabilities are reported through a typed `Capabilities` struct (every field is
a documented boolean), replacing tra86's untyped `BTreeMap<String, bool>`. The
UI greys out actions a backend cannot perform.

## The LLDB backend (`bind-lldb`)

### Why an out-of-process Python driver

On the target platform (arm64 macOS, Apple LLDB) there is **no `liblldb`
development library and no `llvm-config`**, so the `lldb` Rust FFI crate cannot
link. The LLDB **SB API is available from Python** via `PYTHONPATH="$(lldb -P)"`.

Bind therefore hosts the SB API in the interpreter LLDB ships with and talks to
it over a line-delimited JSON protocol:

```
crates/bind-lldb/src/protocol.rs   Rust client (spawns driver, RPC, timeouts)
crates/bind-lldb/driver/bind_lldb_driver.py   embedded SB-API driver
```

This is a **structured programmatic adapter** — it calls the SB API and returns
typed JSON. It never parses the human-readable `lldb` console. This is how Bind
satisfies "no core workflow depends on parsing debugger console output" despite
not being able to link liblldb.

### The protocol

Request: `{"id": N, "op": "<name>", ...args}` on one line.
Response: `{"id": N, "ok": true, "result": <obj>}` or `{"id": N, "ok": false, "error": "<msg>"}`.

Ops: `open_target`, `launch`, `attach`, `resume`, `step`, `add_breakpoint`,
`remove_breakpoint`, `threads`, `frames`, `registers`, `disassemble`,
`read_memory`, `modules`, `process_info`, `detach`, `kill`, `lldb_version`,
`shutdown`. Addresses are integers; instruction/memory bytes are hex strings.

### Thread-safety and timeouts

- Non-thread-safe LLDB (`SB*`) objects never cross the FFI boundary — only
  stable ids and immutable JSON snapshots do. The `SBDebugger` lives entirely in
  the Python process.
- A dedicated reader thread feeds responses over a channel; `request` waits with
  a timeout (`DEFAULT_TIMEOUT = 20s`, tunable). A blocked op returns a typed
  `BindError::Backend` instead of freezing the worker.

### Known platform limitation: live process control

On current macOS, launching/attaching under LLDB requires developer-tools
authorization. In a headless/CI context this **blocks**, so:

- **Static** operations (open target, resolve symbols, resolve breakpoints,
  disassemble) work fully and are covered by a real, non-skipped integration
  test (`tests/lldb_integration.rs`).
- **Live** operations are attempted with a short timeout and, if they do not
  complete, the test **skips with a clear message** rather than hanging or
  silently passing.

To use live debugging locally, ensure debugging is authorized (e.g.
`DevToolsSecurity -enable`, or run in a context where the OS grants the
`com.apple.security.get-task-allow` / debugging entitlement). On Linux with a
full LLDB install, live control works without this dance.

## Adding another backend

Implement `DebugBackend` in a new crate, translate native notifications into
`DebugEvent`s, report honest `Capabilities`, and select it in
`bind-cli`. Nothing else changes.
