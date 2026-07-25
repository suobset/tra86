# Bind architecture

Bind is a Cargo workspace of small, single-responsibility crates whose
dependency arrows all point inward toward `bind-core`. No crate above the
backend references any LLDB type.

## Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `bind-core` | Debugger-independent domain model: IDs, addresses, arch/language metadata, `DebugEvent`, `Command`, `Capabilities`, `SessionSnapshot`, `BindError`. | — |
| `bind-debugger` | `DebugBackend` trait, session orchestration, the worker thread, and `MockBackend`. | core |
| `bind-lldb` | LLDB backend via an out-of-process SB-API Python driver over JSON. | core, debugger |
| `bind-trace` | Bounded ring buffer, versioned trace format, reader/writer, replay, filters. | core |
| `bind-analysis` | Incremental `Analyzer`s emitting confidence-bearing `Finding`s. | core, symbols |
| `bind-symbols` | Address→symbol resolution, demangling, caching. | core |
| `bind-storage` | Human-inspectable config, keybindings, annotations. | core |
| `bind-tui` | Ratatui rendering + input mapping. | core, debugger, trace, analysis, symbols, storage |
| `bind-cli` | Argument parsing, logging, backend selection, wiring, `diag`. | all |

## The three shared abstractions

1. **`SessionSnapshot`** — an immutable, cheap-to-clone read model the TUI
   renders. Produced by the worker on state changes (stops, module loads), not
   per render frame.
2. **`DebugEvent` / `SequencedEvent`** — the single normalized event stream that
   the timeline, trace recorder, and analyzers all consume. Backends translate
   native notifications into these.
3. **`Command`** — typed intents produced by keybindings and the command
   palette and executed by the worker. No core operation is a raw debugger
   console string.

## Concurrency model

```
┌── main thread ────────────┐        ┌── debugger worker thread ─────┐
│ crossterm input           │  Cmd   │ owns Box<dyn DebugBackend>    │
│ UiState (pure)            │──────▶ │ executes commands             │
│ view::render(snapshot)    │        │ drains backend events         │
│                           │◀────── │ folds → SessionSnapshot       │
└───────────────────────────┘ Update └───────────────────────────────┘
        (bind-tui)          mpsc channels     (bind-debugger)
                                              │
                                              ▼ (bind-lldb only)
                                     ┌── reader thread ──┐   ┌── python ──┐
                                     │ JSON lines ◀──────┼───│ SB-API     │
                                     └───────────────────┘   │ driver     │
                                                             └────────────┘
```

- The worker owns the backend; nothing else touches it.
- Requests to the LLDB driver are answered by a dedicated reader thread so they
  can **time out** instead of hanging (critical: live process control can block
  on OS authorization).
- The terminal is restored on every exit path including panic
  (`bind-tui::terminal`).
- Rendering happens only when something changed (input, worker update, resize),
  never on a busy timer.

## Design boundaries the code enforces

- **Architecture knowledge** lives only in `bind_core::arch` (register roles, pc
  / sp names, pointer width). No `if name == "rip"` anywhere else.
- **Demangling** lives only in `bind-symbols`. No Rust/C++ special-casing in UI
  or analysis.
- **LLDB specifics** live only in `bind-lldb`. Everything above sees `bind-core`
  types and normalized events.
- **Failures are surfaced, not swallowed**: data-fetch errors become
  `WorkerUpdate::CommandError` (shown in the UI) or `DebugEvent::Error`.
