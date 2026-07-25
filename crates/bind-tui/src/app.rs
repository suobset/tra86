//! TUI application state and input handling.
//!
//! [`UiState`] is a pure state machine: it folds [`WorkerUpdate`]s into a
//! renderable model and turns key events into typed [`Command`]s (for the
//! worker) while mutating its own view state directly. It performs no I/O and
//! never touches the debugger, which makes every transition unit-testable
//! without a terminal or a backend.

use std::collections::VecDeque;

use bind_analysis::{default_analyzers, AnalysisContext, AnalyzerSet, Finding};
use bind_core::{Address, BreakpointLocation, Command, SequencedEvent, SessionSnapshot, StepKind};
use bind_debugger::WorkerUpdate;
use bind_storage::Preferences;
use bind_symbols::SymbolIndex;
use bind_trace::{TraceMetadata, TraceRecorder};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::palette::{self, LayoutMode, Panel, Parsed, UiAction};

/// How many recent events the interactive timeline retains for display.
const TIMELINE_CAP: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Palette,
    Help,
}

/// A decoded memory view request/result.
#[derive(Debug, Clone, Default)]
pub struct MemoryView {
    pub addr: Address,
    pub len: usize,
    pub bytes: Vec<u8>,
}

pub struct UiState {
    pub snapshot: SessionSnapshot,
    pub timeline: VecDeque<SequencedEvent>,
    pub focus: Panel,
    pub layout: LayoutMode,
    pub mode: Mode,
    pub palette_input: String,
    pub status: String,
    pub error: Option<String>,
    pub help_topic: Option<String>,
    pub selected_timeline: usize,
    pub memory: Option<MemoryView>,
    pub findings: Vec<Finding>,
    pub prefs: Preferences,
    pub should_quit: bool,

    // Analysis plumbing (kept private to the state).
    symbols: SymbolIndex,
    analyzers: AnalyzerSet,
    /// Bounded trace recorder: retains recent events and, when persisting,
    /// accumulates the full record for `trace start/stop`.
    recorder: TraceRecorder,
    /// A memory read the UI has requested and is waiting on.
    pending_memory: Option<(Address, usize)>,
}

impl UiState {
    pub fn new(snapshot: SessionSnapshot, prefs: Preferences) -> Self {
        let recorder = TraceRecorder::new(
            TraceMetadata::new(snapshot.process.arch),
            prefs.ring_capacity,
        );
        Self {
            snapshot,
            timeline: VecDeque::with_capacity(TIMELINE_CAP.min(256)),
            focus: Panel::Code,
            layout: LayoutMode::Mixed,
            mode: Mode::Normal,
            palette_input: String::new(),
            status: "ready".into(),
            error: None,
            help_topic: None,
            selected_timeline: 0,
            memory: None,
            findings: Vec::new(),
            prefs,
            should_quit: false,
            symbols: SymbolIndex::new(),
            analyzers: default_analyzers(),
            recorder,
            pending_memory: None,
        }
    }

    /// Total events observed this session (retained + dropped).
    pub fn trace_total(&self) -> u64 {
        self.recorder.total()
    }

    /// Events not retained in the interactive ring (evicted or filtered out).
    pub fn trace_dropped(&self) -> u64 {
        self.recorder.dropped()
    }

    /// Whether a trace is currently being written to disk.
    pub fn trace_persisting(&self) -> bool {
        self.recorder.is_persisting()
    }

    /// Begins recording a trace (to `path` when given). Used by `--trace` and
    /// the `trace start` palette command.
    pub fn start_trace(&mut self, path: Option<String>) {
        self.recorder
            .metadata_mut()
            .executable
            .clone_from(&self.snapshot.program);
        match path {
            Some(p) => {
                self.recorder.start_persisting(p.clone());
                self.status = format!("recording trace -> {p}");
            }
            None => {
                self.status = "recording trace (in-memory ring)".into();
            }
        }
    }

    /// Folds one worker update into the state.
    pub fn apply_update(&mut self, update: WorkerUpdate) {
        match update {
            WorkerUpdate::Snapshot(snap) => {
                self.ingest_symbols(&snap);
                // Fill in trace metadata once the target's architecture is known.
                let meta = self.recorder.metadata_mut();
                if meta.arch == bind_core::Architecture::Unknown {
                    meta.arch = snap.process.arch;
                }
                if meta.executable.is_none() {
                    meta.executable.clone_from(&snap.program);
                }
                self.snapshot = *snap;
            }
            WorkerUpdate::Event(ev) => {
                let mut ctx = AnalysisContext::new(Some(&mut self.symbols));
                self.analyzers.on_event(&ev, &mut ctx);
                self.findings = self.analyzers.findings();
                self.recorder.record(ev.clone());
                self.push_timeline(ev);
            }
            WorkerUpdate::Memory { addr, bytes } => {
                let len = self
                    .pending_memory
                    .filter(|(a, _)| *a == addr)
                    .map(|(_, l)| l)
                    .unwrap_or(bytes.len());
                self.memory = Some(MemoryView { addr, len, bytes });
                self.pending_memory = None;
                self.focus = Panel::Registers; // memory shares the state column
            }
            WorkerUpdate::CommandError(err) => {
                self.error = Some(format!("[{}] {}", err.category(), err));
            }
            WorkerUpdate::Shutdown => {
                self.status = "debugger stopped".into();
            }
        }
    }

    fn push_timeline(&mut self, ev: SequencedEvent) {
        if self.timeline.len() == TIMELINE_CAP {
            self.timeline.pop_front();
        }
        let follow = self.selected_timeline + 1 >= self.timeline.len();
        self.timeline.push_back(ev);
        if follow {
            self.selected_timeline = self.timeline.len().saturating_sub(1);
        }
    }

    fn ingest_symbols(&mut self, snap: &SessionSnapshot) {
        for insn in &snap.disassembly {
            if let Some(sym) = &insn.symbol {
                if let Some(start) = sym.start {
                    if self.symbols.address_of(&sym.name).is_none() {
                        self.symbols
                            .insert(sym.name.clone(), start, 0, sym.module.clone());
                    }
                }
            }
        }
        for frame in &snap.frames {
            if let (Some(sym), Some(func)) = (&frame.symbol, &frame.function) {
                if let Some(start) = sym.start {
                    if self.symbols.address_of(func).is_none() {
                        self.symbols
                            .insert(func.clone(), start, 0, sym.module.clone());
                    }
                }
            }
        }
    }

    /// Handles a key event, returning any worker commands to dispatch.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Command> {
        // Ctrl-C always quits, in any mode.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return vec![];
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Palette => self.handle_palette(key),
            Mode::Help => {
                // Any key dismisses help.
                self.mode = Mode::Normal;
                vec![]
            }
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Vec<Command> {
        self.error = None;
        match key.code {
            KeyCode::Char(':') => {
                self.mode = Mode::Palette;
                self.palette_input.clear();
                vec![]
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                vec![]
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
                vec![]
            }
            KeyCode::Tab => {
                self.focus = next_panel(self.focus);
                vec![]
            }
            KeyCode::BackTab => {
                self.focus = prev_panel(self.focus);
                vec![]
            }
            KeyCode::Char('c') => vec![Command::Continue],
            KeyCode::Char('s') => vec![Command::Step(StepKind::Into)],
            KeyCode::Char('n') => vec![Command::Step(StepKind::Over)],
            KeyCode::Char('o') => vec![Command::Step(StepKind::Out)],
            KeyCode::Char('i') => vec![Command::Step(StepKind::Instruction)],
            KeyCode::Char('p') => vec![Command::Pause],
            KeyCode::Char('b') => self.toggle_breakpoint_at_current(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                vec![]
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                vec![]
            }
            _ => vec![],
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.focus == Panel::Timeline && !self.timeline.is_empty() {
            let max = self.timeline.len() - 1;
            let cur = self.selected_timeline as isize + delta;
            self.selected_timeline = cur.clamp(0, max as isize) as usize;
        }
    }

    fn handle_palette(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.palette_input.clear();
                vec![]
            }
            KeyCode::Backspace => {
                self.palette_input.pop();
                vec![]
            }
            KeyCode::Char(c) => {
                self.palette_input.push(c);
                vec![]
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.palette_input);
                self.mode = Mode::Normal;
                self.run_palette(&input)
            }
            _ => vec![],
        }
    }

    /// Parses and applies a palette line, returning worker commands.
    pub fn run_palette(&mut self, input: &str) -> Vec<Command> {
        match palette::parse(input) {
            Ok(None) => vec![],
            Ok(Some(Parsed::Command(cmd))) => {
                self.status = format!("> {input}");
                vec![cmd]
            }
            Ok(Some(Parsed::Ui(action))) => self.apply_ui_action(action),
            Err(msg) => {
                self.error = Some(msg);
                vec![]
            }
        }
    }

    fn apply_ui_action(&mut self, action: UiAction) -> Vec<Command> {
        match action {
            UiAction::Quit => {
                self.should_quit = true;
                vec![]
            }
            UiAction::Help(topic) => {
                self.help_topic = topic;
                self.mode = Mode::Help;
                vec![]
            }
            UiAction::Layout(mode) => {
                self.layout = mode;
                vec![]
            }
            UiAction::FocusPanel(p) => {
                self.focus = p;
                vec![]
            }
            UiAction::Search(term) => {
                self.status = format!("search: {term}");
                vec![]
            }
            UiAction::ShowMemory { addr, len } => {
                self.pending_memory = Some((addr, len));
                self.status = format!("reading {len} bytes at {addr}");
                vec![Command::ReadMemory { addr, len }]
            }
            UiAction::TraceStart { path } => {
                self.start_trace(path);
                vec![]
            }
            UiAction::TraceStop => {
                match self.recorder.stop_persisting() {
                    Ok(Some(p)) => self.status = format!("trace saved: {}", p.display()),
                    Ok(None) => self.status = "trace stopped (was not persisting)".into(),
                    Err(e) => self.error = Some(e.to_string()),
                }
                vec![]
            }
        }
    }

    /// Toggles a breakpoint at the current instruction (or the selected frame's
    /// pc): removes an existing one there, otherwise adds one by address.
    fn toggle_breakpoint_at_current(&mut self) -> Vec<Command> {
        let pc = self
            .snapshot
            .disassembly
            .iter()
            .find(|i| i.is_current)
            .map(|i| i.address)
            .or_else(|| self.snapshot.selected_frame().map(|f| f.pc));
        let Some(pc) = pc else {
            self.error = Some("no current instruction to toggle a breakpoint on".into());
            return vec![];
        };
        if let Some(bp) = self
            .snapshot
            .breakpoints
            .iter()
            .find(|b| b.resolved.iter().any(|r| r.address == pc))
        {
            self.status = format!("removing breakpoint #{} at {pc}", bp.id);
            vec![Command::RemoveBreakpoint(bp.id)]
        } else {
            self.status = format!("breakpoint at {pc}");
            vec![Command::AddBreakpoint {
                location: BreakpointLocation::Address(pc),
                condition: None,
            }]
        }
    }

    pub fn selected_event(&self) -> Option<&SequencedEvent> {
        self.timeline.get(self.selected_timeline)
    }
}

fn next_panel(p: Panel) -> Panel {
    match p {
        Panel::Code => Panel::Registers,
        Panel::Registers => Panel::Stack,
        Panel::Stack => Panel::Timeline,
        Panel::Timeline => Panel::Code,
    }
}

fn prev_panel(p: Panel) -> Panel {
    match p {
        Panel::Code => Panel::Timeline,
        Panel::Registers => Panel::Code,
        Panel::Stack => Panel::Registers,
        Panel::Timeline => Panel::Stack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_core::{Capabilities, SessionId};

    fn state() -> UiState {
        let snap = SessionSnapshot::empty(SessionId::new(1), "mock", Capabilities::NONE);
        UiState::new(snap, Preferences::default())
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn palette_opens_and_parses() {
        let mut s = state();
        s.handle_key(key(':'));
        assert_eq!(s.mode, Mode::Palette);
        for c in "continue".chars() {
            s.handle_key(key(c));
        }
        assert_eq!(s.palette_input, "continue");
        let cmds = s.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(cmds, vec![Command::Continue]);
        assert_eq!(s.mode, Mode::Normal);
    }

    #[test]
    fn focus_cycles_with_tab() {
        let mut s = state();
        assert_eq!(s.focus, Panel::Code);
        s.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(s.focus, Panel::Registers);
    }

    #[test]
    fn step_key_emits_command() {
        let mut s = state();
        assert_eq!(
            s.handle_key(key('i')),
            vec![Command::Step(StepKind::Instruction)]
        );
    }

    #[test]
    fn ctrl_c_quits_from_any_mode() {
        let mut s = state();
        s.mode = Mode::Palette;
        s.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(s.should_quit);
    }

    #[test]
    fn bad_palette_input_sets_error_not_panic() {
        let mut s = state();
        let cmds = s.run_palette("frobnicate");
        assert!(cmds.is_empty());
        assert!(s.error.is_some());
    }

    #[test]
    fn memory_request_is_deferred_command() {
        let mut s = state();
        let cmds = s.run_palette("memory 0x1000 32");
        assert_eq!(cmds.len(), 1);
        matches!(cmds[0], Command::ReadMemory { .. });
    }
}
