//! Noninteractive diagnostic driver.
//!
//! `bind diag` runs a scripted debugging flow with no TUI and prints a report.
//! It is the end-to-end smoke harness (spec §14.4): build a fixture, launch,
//! breakpoint, continue, observe a stop, read registers/frames, step, and exit
//! cleanly — all against whichever backend is selected. Factored into a pure
//! function so it is unit-testable against the mock backend.

use std::fmt::Write as _;

use bind_core::{AttachSpec, BreakpointLocation, Command, SessionId, StepKind, TargetSpec};
use bind_debugger::{DebugBackend, WorkerHandle, WorkerUpdate};

/// Drives a fixed debugging script against `backend`, returning a human report.
/// `program` is launched; `breakpoint_symbol` is set before continuing.
pub fn run_diag(backend: Box<dyn DebugBackend>, program: &str, breakpoint_symbol: &str) -> String {
    let mut out = String::new();
    let backend_name = backend.name();
    let _ = writeln!(out, "bind diag — backend: {backend_name}");

    let worker = WorkerHandle::spawn(SessionId::new(1), backend);

    let script = [
        Command::Attach(AttachSpec::Launch(TargetSpec {
            program: program.to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            stop_at_entry: true,
        })),
        Command::AddBreakpoint {
            location: BreakpointLocation::Symbol(breakpoint_symbol.to_string()),
            condition: None,
        },
        Command::Continue,
        Command::Step(StepKind::Instruction),
    ];

    for cmd in script {
        let label = format!("{cmd:?}");
        if worker.send(cmd).is_err() {
            let _ = writeln!(out, "  ! worker disconnected");
            break;
        }
        // Give the worker a moment, then drain.
        std::thread::sleep(std::time::Duration::from_millis(60));
        for update in worker.drain() {
            match update {
                WorkerUpdate::Event(ev) => {
                    let _ = writeln!(out, "  event #{} {}", ev.seq.raw(), ev.event.kind());
                }
                WorkerUpdate::Snapshot(snap) => {
                    let _ = writeln!(
                        out,
                        "  snapshot: {:?} pc-frame={} regs={} disasm={} bps={}",
                        snap.process.lifecycle,
                        snap.selected_frame()
                            .and_then(|f| f.function.clone())
                            .unwrap_or_else(|| "?".into()),
                        snap.registers
                            .as_ref()
                            .map(|r| r.registers.len())
                            .unwrap_or(0),
                        snap.disassembly.len(),
                        snap.breakpoints.len(),
                    );
                }
                WorkerUpdate::CommandError(err) => {
                    let _ = writeln!(out, "  error [{}]: {}", err.category(), err);
                }
                WorkerUpdate::Memory { .. } => {}
                WorkerUpdate::Shutdown => {}
            }
        }
        let _ = writeln!(out, "  (after {label})");
    }

    worker.shutdown();
    let _ = writeln!(out, "diag complete");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_debugger::MockBackend;

    #[test]
    fn diag_runs_full_script_on_mock() {
        let report = run_diag(Box::new(MockBackend::default()), "mock", "helper");
        assert!(report.contains("backend: mock"));
        // Should observe a launch, a breakpoint hit, and reach `helper`.
        assert!(report.contains("process-launched"), "report:\n{report}");
        assert!(report.contains("breakpoint-hit"), "report:\n{report}");
        assert!(report.contains("helper"), "report:\n{report}");
        assert!(report.contains("diag complete"));
    }
}
