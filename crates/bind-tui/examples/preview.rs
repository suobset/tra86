//! Renders a populated mock debugging session to an off-screen buffer and
//! prints it, so the layout can be eyeballed without a live terminal.
//!
//! Run: `cargo run -p bind-tui --example preview`

use std::time::Duration;

use bind_core::{
    AttachSpec, BreakpointLocation, Capabilities, Command, SessionId, SessionSnapshot, StepKind,
    TargetSpec,
};
use bind_debugger::{MockBackend, WorkerHandle};
use bind_storage::Preferences;
use bind_tui::{render, UiState};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn main() {
    let worker = WorkerHandle::spawn(SessionId::new(1), Box::new(MockBackend::default()));
    let mut state = UiState::new(
        SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE),
        Preferences::default(),
    );
    state.snapshot.program = Some("./demo".into());

    // Drive a small scripted session: launch, break at helper, continue, step.
    let script = [
        Command::Attach(AttachSpec::Launch(TargetSpec {
            program: "./demo".into(),
            args: vec![],
            cwd: None,
            env: vec![],
            stop_at_entry: true,
        })),
        Command::AddBreakpoint {
            location: BreakpointLocation::Symbol("helper".into()),
            condition: None,
        },
        Command::Continue,
        Command::Step(StepKind::Instruction),
        Command::Step(StepKind::Instruction),
    ];
    for cmd in script {
        worker.send(cmd).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        for u in worker.drain() {
            state.apply_update(u);
        }
    }

    let (w, h) = (110u16, 34u16);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, &state)).unwrap();
    let buf = term.backend().buffer().clone();

    println!("+{}+", "-".repeat(w as usize));
    for y in 0..h {
        let mut row = String::new();
        for x in 0..w {
            row.push_str(buf[(x, y)].symbol());
        }
        println!("|{row}|");
    }
    println!("+{}+", "-".repeat(w as usize));

    worker.shutdown();
}
