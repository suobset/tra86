# Trace format

Bind records the normalized event stream so a session can be inspected and
replayed with no live debugger. The format is plain serde/JSON of `bind-core`
types — never a serialized backend object graph.

## Layout

A `TraceRecord` is:

```jsonc
{
  "metadata": {
    "schema_version": 1,
    "bind_version": "0.1.0",
    "lldb_version": "lldb-....",     // optional
    "host_os": "macos",
    "arch": "Aarch64",
    "executable": "./demo",           // optional
    "executable_hash": null,          // optional
    "command_line": ["./demo", "arg"],
    "environment": [["PATH", "..."]], // secrets stripped, see below
    "modules": ["demo", "libSystem.B.dylib"],
    "started_at": "...", "ended_at": "...",
    "policy": { "filter_description": null, "sample_rate": 0, "retention_capacity": 50000 },
    "dropped_events": 0
  },
  "events": [ { "seq": 1, "mono_nanos": 12345, "wall": "...", "event": { "Stopped": { ... } } } ],
  "annotations": []
}
```

Each event carries a monotonic `seq`, a monotonic `mono_nanos` timestamp, an
optional wall-clock time, and the typed `DebugEvent` payload (which carries
process/thread/address context as applicable).

## Forwards evolution

- Readers do **not** use `deny_unknown_fields`; every added field has
  `#[serde(default)]`. A trace written by a newer Bind opens in an older Bind,
  ignoring unknown fields.
- `schema_version` guards against *misinterpreting* an incompatible major
  version: a reader refuses a version newer than it understands rather than
  guessing. (`read_trace` returns `BindError::Trace` in that case.)

## Bounded capture

- Interactive history lives in a `RingBuffer` (default 50,000 events) that
  **counts what it evicts**; the UI shows "N shown, D dropped" so a truncated
  trace is never presented as complete.
- `TraceFilter` selects events by kind, thread, or address range before they are
  retained. Filtered-out events are counted too.
- Persisting (`trace start <path>`) writes the full accumulated record on
  `trace stop`, atomically via a temp file + rename.

## Secrets

`sanitize_env` drops environment variables whose names contain `TOKEN`,
`SECRET`, `PASSWORD`, `KEY`, `CREDENTIAL`, `AUTH`, `SESSION`, etc., so secrets
are never persisted into a trace.

## Replay

`Replay` walks a loaded record's events in order with no debugger present:

```rust
let mut replay = bind_trace::Replay::from_path("session.bindtrace")?;
while let Some(ev) = replay.next_event() { /* feed analyzers, drive a timeline */ }
```
