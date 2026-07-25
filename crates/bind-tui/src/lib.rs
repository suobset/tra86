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
pub use run::{run, run_app};
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
