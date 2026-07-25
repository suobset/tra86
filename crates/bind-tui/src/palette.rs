//! The typed command palette.
//!
//! Text typed into the palette (or a keybinding) is parsed here into either a
//! [`bind_core::Command`] for the worker or a [`UiAction`] handled in the TUI.
//! Nothing here concatenates strings into a debugger console — every core
//! operation maps to a typed command. Parse errors are returned as messages,
//! never panics.

use bind_core::{Address, BreakpointLocation, Command, RunTarget, StepKind};

/// Actions handled entirely within the TUI (no debugger involvement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    Quit,
    Help(Option<String>),
    Layout(LayoutMode),
    FocusPanel(Panel),
    Search(String),
    ShowMemory {
        addr: Address,
        len: usize,
    },
    /// Begin recording the event stream (to a file when `path` is given).
    TraceStart {
        path: Option<String>,
    },
    TraceStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Source and disassembly side by side (default).
    Mixed,
    /// Source only.
    Source,
    /// Disassembly only.
    Disassembly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Code,
    Registers,
    Stack,
    Timeline,
}

/// The parsed result of a palette line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Command(Command),
    Ui(UiAction),
}

/// The set of command names, used for the palette's fuzzy suggestions.
pub const COMMAND_NAMES: &[&str] = &[
    "continue",
    "pause",
    "step",
    "next",
    "finish",
    "step-instruction",
    "break",
    "delete-breakpoint",
    "thread",
    "frame",
    "memory",
    "goto",
    "trace",
    "layout",
    "search",
    "help",
    "quit",
];

fn parse_address(tok: &str) -> Result<Address, String> {
    let t = tok.trim();
    let value = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        // Accept bare hex too, since addresses are conventionally hex.
        u64::from_str_radix(t, 16).or_else(|_| t.parse::<u64>())
    };
    value
        .map(Address::new)
        .map_err(|_| format!("invalid address: {tok}"))
}

/// Parses one palette command line. Returns `Ok(None)` for a blank line.
pub fn parse(line: &str) -> Result<Option<Parsed>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap();
    let rest: Vec<&str> = parts.collect();

    let parsed = match cmd {
        "continue" | "c" => Parsed::Command(Command::Continue),
        "pause" => Parsed::Command(Command::Pause),
        "step" | "s" => Parsed::Command(Command::Step(StepKind::Into)),
        "next" | "n" => Parsed::Command(Command::Step(StepKind::Over)),
        "finish" | "o" => Parsed::Command(Command::Step(StepKind::Out)),
        "step-instruction" | "si" | "i" => Parsed::Command(Command::Step(StepKind::Instruction)),
        "break" | "b" => parse_break(&rest)?,
        "delete-breakpoint" | "d" => {
            let id = rest
                .first()
                .ok_or("usage: delete-breakpoint <id>")?
                .parse::<u64>()
                .map_err(|_| "breakpoint id must be a number".to_string())?;
            Parsed::Command(Command::RemoveBreakpoint(bind_core::BreakpointId::new(id)))
        }
        "thread" => {
            let id = rest
                .first()
                .ok_or("usage: thread <id>")?
                .parse::<u64>()
                .map_err(|_| "thread id must be a number".to_string())?;
            Parsed::Command(Command::SelectThread(bind_core::ThreadId::new(id)))
        }
        "frame" => {
            let id = rest
                .first()
                .ok_or("usage: frame <index>")?
                .parse::<u64>()
                .map_err(|_| "frame index must be a number".to_string())?;
            Parsed::Command(Command::SelectFrame(bind_core::FrameId::new(id)))
        }
        "memory" | "mem" | "x" => {
            let addr = parse_address(rest.first().ok_or("usage: memory <addr> [len]")?)?;
            let len = rest
                .get(1)
                .map(|s| s.parse::<usize>())
                .transpose()
                .map_err(|_| "length must be a number".to_string())?
                .unwrap_or(128);
            Parsed::Ui(UiAction::ShowMemory { addr, len })
        }
        "goto" | "g" => {
            let addr = parse_address(rest.first().ok_or("usage: goto <addr>")?)?;
            Parsed::Command(Command::RunTo(RunTarget::Address(addr)))
        }
        "trace" => parse_trace(&rest)?,
        "layout" => {
            let mode = match rest.first().copied() {
                Some("source") => LayoutMode::Source,
                Some("disassembly") | Some("disasm") | Some("asm") => LayoutMode::Disassembly,
                Some("mixed") | None => LayoutMode::Mixed,
                Some(other) => return Err(format!("unknown layout: {other}")),
            };
            Parsed::Ui(UiAction::Layout(mode))
        }
        "search" | "/" => Parsed::Ui(UiAction::Search(rest.join(" "))),
        "help" | "?" => Parsed::Ui(UiAction::Help(rest.first().map(|s| s.to_string()))),
        "quit" | "q" | "exit" => Parsed::Ui(UiAction::Quit),
        other => return Err(format!("unknown command: {other}")),
    };
    Ok(Some(parsed))
}

fn parse_source_spec(spec: &str) -> Result<BreakpointLocation, String> {
    let (file, line) = spec
        .rsplit_once(':')
        .ok_or("usage: break file <path>:<line>")?;
    Ok(BreakpointLocation::Source {
        file: file.to_string(),
        line: line.parse().map_err(|_| "invalid line".to_string())?,
    })
}

fn parse_break(rest: &[&str]) -> Result<Parsed, String> {
    if rest.is_empty() {
        return Err("usage: break <symbol> | break address 0x.. | break file f:line".into());
    }
    let location = match rest[0] {
        "symbol" => {
            let name = rest.get(1).ok_or("usage: break symbol <name>")?;
            BreakpointLocation::Symbol((*name).to_string())
        }
        "address" => {
            let addr = parse_address(rest.get(1).ok_or("usage: break address 0x..")?)?;
            BreakpointLocation::Address(addr)
        }
        "file" => parse_source_spec(rest.get(1).ok_or("usage: break file <path>:<line>")?)?,
        // Shorthands: `break main`, `break 0x1000`, `break file.rs:42`.
        other if other.starts_with("0x") => BreakpointLocation::Address(parse_address(other)?),
        other if other.contains(':') => parse_source_spec(other)?,
        other => BreakpointLocation::Symbol(other.to_string()),
    };
    Ok(Parsed::Command(Command::AddBreakpoint {
        location,
        condition: None,
    }))
}

fn parse_trace(rest: &[&str]) -> Result<Parsed, String> {
    // Trace capture lives in the TUI (it owns the event ring), so these are UI
    // actions, not worker commands.
    match rest.first().copied() {
        Some("start") => Ok(Parsed::Ui(UiAction::TraceStart {
            path: rest.get(1).map(|s| s.to_string()),
        })),
        Some("stop") => Ok(Parsed::Ui(UiAction::TraceStop)),
        _ => Err("usage: trace start [path] | trace stop".into()),
    }
}

/// Ranks command names by a simple subsequence/prefix match for palette hints.
pub fn suggestions(prefix: &str) -> Vec<&'static str> {
    let p = prefix.trim().to_ascii_lowercase();
    if p.is_empty() {
        return COMMAND_NAMES.to_vec();
    }
    let mut scored: Vec<(&'static str, u8)> = COMMAND_NAMES
        .iter()
        .filter_map(|name| {
            if name.starts_with(&p) {
                Some((*name, 0))
            } else if name.contains(&p) {
                Some((*name, 1))
            } else if is_subsequence(&p, name) {
                Some((*name, 2))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
    scored.into_iter().map(|(n, _)| n).collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|c| chars.any(|h| h == c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_execution_commands() {
        assert_eq!(
            parse("continue").unwrap(),
            Some(Parsed::Command(Command::Continue))
        );
        assert_eq!(
            parse("si").unwrap(),
            Some(Parsed::Command(Command::Step(StepKind::Instruction)))
        );
        assert_eq!(
            parse("n").unwrap(),
            Some(Parsed::Command(Command::Step(StepKind::Over)))
        );
    }

    #[test]
    fn parses_breakpoints() {
        assert_eq!(
            parse("break main").unwrap(),
            Some(Parsed::Command(Command::AddBreakpoint {
                location: BreakpointLocation::Symbol("main".into()),
                condition: None,
            }))
        );
        assert_eq!(
            parse("break address 0x1000").unwrap(),
            Some(Parsed::Command(Command::AddBreakpoint {
                location: BreakpointLocation::Address(Address::new(0x1000)),
                condition: None,
            }))
        );
        assert_eq!(
            parse("break file src/main.rs:42").unwrap(),
            Some(Parsed::Command(Command::AddBreakpoint {
                location: BreakpointLocation::Source {
                    file: "src/main.rs".into(),
                    line: 42
                },
                condition: None,
            }))
        );
    }

    #[test]
    fn parses_memory_and_goto() {
        assert_eq!(
            parse("memory 0x7fff 64").unwrap(),
            Some(Parsed::Ui(UiAction::ShowMemory {
                addr: Address::new(0x7fff),
                len: 64
            }))
        );
        assert_eq!(
            parse("goto 0x1004").unwrap(),
            Some(Parsed::Command(Command::RunTo(RunTarget::Address(
                Address::new(0x1004)
            ))))
        );
    }

    #[test]
    fn blank_and_unknown() {
        assert_eq!(parse("   ").unwrap(), None);
        assert!(parse("frobnicate").is_err());
    }

    #[test]
    fn suggestions_prefix_and_subsequence() {
        let s = suggestions("st");
        assert_eq!(s.first(), Some(&"step"));
        assert!(suggestions("cont").contains(&"continue"));
        // subsequence: "brk" -> "break"
        assert!(suggestions("brk").contains(&"break"));
    }
}
