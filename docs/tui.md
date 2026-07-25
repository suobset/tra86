# The TUI

Bind's interface is a Ratatui terminal application. It renders read-only
`SessionSnapshot`s and dispatches typed `Command`s; it contains no debugger
logic.

## Layout

```
┌ Bind — program, pid, lifecycle, arch, stop reason, backend ─────────────────┐
├───────────────────────────────────────┬────────────────────────────────────┤
│ Source / Disassembly                  │ Registers (changed values marked)  │
│  ▶ current instruction, breakpoints    ├────────────────────────────────────┤
├───────────────────────────────────────┤ Memory (hex + ASCII)               │
│ Stack / Frames                        │                                    │
├───────────────────────────────────────┴────────────────────────────────────┤
│ Timeline (events)                     │ Analysis (findings w/ confidence)  │
├─────────────────────────────────────────────────────────────────────────────┤
│ status / command palette / key hints / errors                               │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Keys

| Key | Action | Key | Action |
|---|---|---|---|
| `c` | continue | `n` | step over |
| `s` | step into | `o` | finish (step out) |
| `i` | step instruction | `p` | pause (if supported) |
| `Tab` / `Shift-Tab` | cycle focus | `j`/`k`/↑/↓ | move selection |
| `:` | command palette | `/` | search |
| `?` | help | `q` / `Ctrl-C` | quit |

## Command palette (`:`)

Typed commands, parsed in `bind_tui::palette`, never concatenated into a
debugger console:

```
continue | pause | step | next | finish | step-instruction
break <symbol> | break address 0x.. | break file f.rs:42 | delete-breakpoint <id>
thread <id> | frame <index> | memory <addr> [len] | goto <addr>
trace start [path] | trace stop | layout source|disassembly|mixed
search <term> | help [topic] | quit
```

The status bar shows fuzzy suggestions as you type. Parse errors are shown
inline; they never panic.

## Degradation & safety

- **Small terminals:** every panel guards against tiny/zero areas; rendering is
  best-effort but never panics (tested down to 1×1).
- **No Unicode:** `--no-unicode` (or `preferences.unicode = false`) switches
  glyphs (`▶ ● ◆ ·`) to ASCII (`> * # .`).
- **Monochrome:** the `Monochrome` theme uses reverse-video instead of color.
- **Missing data:** empty panels show a hint ("no code — launch or attach…")
  rather than blank space or a crash.
- **Panic safety:** `bind_tui::terminal::install_panic_hook` restores the
  terminal before any panic message prints; `TerminalGuard` restores on drop.

## Testing

`UiState` is a pure state machine and `view::render` is a pure projection, so
both are tested with no real terminal — the render tests draw into ratatui's
`TestBackend` and assert on the resulting cell buffer. See `docs/testing.md`.
