//! `bind-tui`: the terminal user interface.
//!
//! The TUI dispatches typed [`bind_core::Command`]s and renders read-only
//! [`bind_core::SessionSnapshot`]s; it contains no debugger logic. [`UiState`]
//! is a pure state machine (testable without a terminal) and [`view::render`]
//! is a pure projection onto a ratatui frame (testable with `TestBackend`).

pub mod app;
pub mod palette;
pub mod run;
pub mod terminal;
pub mod theme;
pub mod view;

pub use app::{Mode, UiState};
pub use palette::{LayoutMode, Panel};
pub use run::{run, run_app, CrosstermInput, InputSource, ScriptedInput};
pub use view::render;

#[cfg(test)]
mod render_tests {
    use super::*;
    use bind_core::{
        Address, Capabilities, DebugEvent, EventSeq, Instruction, ProcessInfo, ProcessLifecycle,
        Register, RegisterChange, RegisterRole, RegisterSet, SequencedEvent, SessionId,
        SessionSnapshot, StopReason, Symbol, ThreadId,
    };
    use bind_debugger::WorkerUpdate;
    use bind_storage::Preferences;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_text(state: &UiState, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, state)).unwrap();
        let buf = term.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect::<String>()
    }

    fn populated_snapshot() -> SessionSnapshot {
        let mut snap = SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE);
        snap.program = Some("./demo".into());
        snap.process = ProcessInfo {
            lifecycle: ProcessLifecycle::Stopped,
            arch: bind_core::Architecture::Aarch64,
            stop_reason: StopReason::Step,
            ..ProcessInfo::default()
        };
        snap.disassembly = vec![Instruction {
            address: Address::new(0x1000),
            bytes: vec![0x00, 0x00, 0x80, 0xd2],
            mnemonic: "mov".into(),
            operands: "x0, #0".into(),
            symbol: Some(Symbol {
                name: "main".into(),
                demangled: None,
                module: None,
                start: Some(Address::new(0x1000)),
                offset: 0,
            }),
            source: None,
            branch_target: None,
            is_current: true,
            has_breakpoint: false,
        }];
        snap.registers = Some(RegisterSet {
            thread_id: ThreadId::new(1),
            registers: vec![
                Register::new("x0", 7, 64, RegisterRole::General),
                Register::new("sp", 0x7fff_0000, 64, RegisterRole::StackPointer),
            ],
        });
        snap.register_changes = vec![RegisterChange {
            name: "x0".into(),
            old_value: 0,
            new_value: 7,
        }];
        snap
    }

    #[test]
    fn renders_populated_at_standard_size() {
        let state = UiState::new(populated_snapshot(), Preferences::default());
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("Bind"));
        assert!(text.contains("mov"));
        assert!(text.contains("Registers"));
        assert!(text.contains("Timeline"));
    }

    #[test]
    fn renders_empty_state_without_panic() {
        let snap = SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE);
        let state = UiState::new(snap, Preferences::default());
        let text = buffer_text(&state, 80, 24);
        assert!(text.contains("no code"));
    }

    #[test]
    fn renders_in_tiny_terminal_without_panic() {
        let state = UiState::new(populated_snapshot(), Preferences::default());
        // Absurdly small: must not panic, output is best-effort.
        let _ = buffer_text(&state, 12, 6);
        let _ = buffer_text(&state, 1, 1);
    }

    #[test]
    fn ascii_fallback_when_unicode_disabled() {
        let prefs = Preferences {
            unicode: false,
            ..Preferences::default()
        };
        let state = UiState::new(populated_snapshot(), prefs);
        let text = buffer_text(&state, 100, 30);
        // The current-instruction marker should be the ASCII '>' not '▶'.
        assert!(!text.contains('▶'));
    }

    #[test]
    fn help_overlay_renders() {
        let mut state = UiState::new(populated_snapshot(), Preferences::default());
        state.mode = Mode::Help;
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("Help"));
        assert!(text.contains("continue"));
    }

    #[test]
    fn error_is_shown_in_status() {
        let mut state = UiState::new(populated_snapshot(), Preferences::default());
        state.apply_update(WorkerUpdate::CommandError(bind_core::BindError::UserInput(
            "bad address".into(),
        )));
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("error"));
        assert!(text.contains("bad address"));
    }

    #[test]
    fn timeline_shows_events() {
        let mut state = UiState::new(populated_snapshot(), Preferences::default());
        state.apply_update(WorkerUpdate::Event(SequencedEvent::new(
            EventSeq::new(1),
            0,
            DebugEvent::Stopped {
                thread: ThreadId::new(1),
                reason: StopReason::Step,
                pc: Some(Address::new(0x1000)),
            },
        )));
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("stopped"));
    }
}

/// End-to-end loop tests: drive the *actual* `run` event loop against a real
/// worker + mock backend using scripted input, rendering into a `TestBackend`.
/// This exercises the full input → command → worker → update → render pipeline
/// with no real terminal.
#[cfg(test)]
mod loop_tests {
    use super::*;
    use bind_core::{AttachSpec, Capabilities, SessionId, SessionSnapshot, TargetSpec};
    use bind_debugger::{MockBackend, WorkerHandle};
    use bind_storage::Preferences;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::Duration;

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn full_loop_launch_step_quit_against_mock() {
        let worker = WorkerHandle::spawn(SessionId::new(1), Box::new(MockBackend::default()));
        worker
            .send(bind_core::Command::Attach(AttachSpec::Launch(
                TargetSpec::program("mock"),
            )))
            .unwrap();
        // Let the worker produce the launch/stop snapshot before the loop runs.
        std::thread::sleep(Duration::from_millis(150));

        let snap = SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE);
        let mut state = UiState::new(snap, Preferences::default());

        // Script: step an instruction, step again, then quit.
        let mut input = ScriptedInput::new([key('i'), key('i'), key('q')]);
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();

        run(&mut term, &mut state, &worker, &mut input).unwrap();

        assert!(state.should_quit, "loop should have quit on 'q'");
        // The launch snapshot must have flowed through the loop into UI state.
        assert!(
            state.snapshot.process.lifecycle.is_stopped(),
            "expected a stopped process, got {:?}",
            state.snapshot.process.lifecycle
        );
        assert!(
            !state.snapshot.disassembly.is_empty(),
            "disassembly should be populated after launch"
        );
        // Events must have been recorded into the trace ring.
        assert!(state.trace_total() > 0, "expected recorded trace events");

        worker.shutdown();
    }

    #[test]
    fn trace_start_stop_persists_via_state() {
        let dir = std::env::temp_dir().join(format!("bindtui-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.bindtrace");

        let mut snap = SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE);
        snap.program = Some("demo".into());
        let mut state = UiState::new(snap, Preferences::default());

        // Start persisting, feed an event through the normal update path, stop.
        state.start_trace(Some(path.to_string_lossy().to_string()));
        assert!(state.trace_persisting());
        state.apply_update(bind_debugger::WorkerUpdate::Event(
            bind_core::SequencedEvent::new(
                bind_core::EventSeq::new(1),
                0,
                bind_core::DebugEvent::Continued,
            ),
        ));
        let cmds = state.run_palette("trace stop");
        assert!(cmds.is_empty());
        assert!(!state.trace_persisting());

        // The persisted file should be a readable, compatible trace.
        let record = bind_trace::read_trace(&path).expect("trace should be readable");
        assert!(record.metadata.is_compatible());
        assert!(!record.events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
