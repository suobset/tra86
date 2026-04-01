use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use eframe::egui;
use serde::{Deserialize, Serialize};
use tra86_analysis::{build_delta, compute_register_delta};
use tra86_backend::{BackendOrchestrator, DebugBackend, LaunchRequest, MockBackend};
use tra86_backend_lldb::LldbBackend;
use tra86_core::{
    Breakpoint, BreakpointLocation, DisassemblyLine, InstructionRecord, RegisterBank, StopReason,
    SymbolInfo, TargetBinary, TraceEvent,
};
use tra86_ui::{BackendChoice, SessionPhase, SessionStatus, UiEvent, UiModel};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "tra86",
        options,
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::<Tra86App>::default())
        }),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    Ok(())
}

fn configure_theme(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
}

struct Tra86App {
    ui: UiModel,
    worker: BackendWorker,
    previous_registers: Option<RegisterBank>,
    previous_sp: Option<i64>,
    last_traced_ip: Option<u64>,
    trace: Vec<TraceEvent>,
    recents: RecentSessions,
}

impl Default for Tra86App {
    fn default() -> Self {
        let recents = RecentSessions::load();
        let mut ui = UiModel::default();
        if let Some(first) = recents.items.first() {
            ui.executable_path = first.program.clone();
            ui.launch_args = first.args.join(" ");
        }

        let worker = BackendWorker::spawn(BackendChoice::Lldb);

        Self {
            ui,
            worker,
            previous_registers: None,
            previous_sp: None,
            last_traced_ip: None,
            trace: Vec::new(),
            recents,
        }
    }
}

impl eframe::App for Tra86App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Some(msg) = self.worker.try_recv() {
            self.apply_worker_message(msg);
        }

        let events = tra86_ui::render(ctx, &mut self.ui);
        for event in events {
            self.handle_event(event);
        }

        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

impl Tra86App {
    fn reset_trace_for_new_session(&mut self) {
        self.previous_registers = None;
        self.previous_sp = None;
        self.last_traced_ip = None;
        self.trace.clear();
        self.ui.trace.clear();
        self.ui.register_diffs.clear();
    }

    fn clear_live_views(&mut self) {
        self.ui.threads.clear();
        self.ui.frames.clear();
        self.ui.registers = None;
        self.ui.register_diffs.clear();
        self.ui.memory_map_lines.clear();
        self.ui.memory_bytes.clear();
    }

    fn queue_backend_command(&mut self, command: BackendCommand, pending_detail: impl Into<String>) {
        self.ui.session.is_busy = true;
        self.ui.session.last_error = None;
        self.ui.status_line = pending_detail.into();
        self.worker.send(command);
    }

    fn push_ui_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.ui.session.last_error = Some(message.clone());
        self.ui.output_lines.push(format!("error: {message}"));
    }

    fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::SetBackend(choice) => {
                self.ui.backend_choice = choice;
                self.reset_trace_for_new_session();
                self.clear_live_views();
                self.queue_backend_command(
                    BackendCommand::SwitchBackend(choice),
                    format!("Switching backend to {choice:?}"),
                );
                if !self.ui.executable_path.trim().is_empty() {
                    self.queue_backend_command(
                        BackendCommand::OpenTarget(self.ui.executable_path.clone()),
                        "Loading target into new backend",
                    );
                }
            }
            UiEvent::OpenExecutablePicker => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.ui.executable_path = path.display().to_string();
                    self.ui
                        .output_lines
                        .push(format!("Configured target: {}", self.ui.executable_path));
                    self.reset_trace_for_new_session();
                    self.clear_live_views();
                    self.queue_backend_command(
                        BackendCommand::OpenTarget(self.ui.executable_path.clone()),
                        "Loading target",
                    );
                }
            }
            UiEvent::Launch => {
                if self.ui.session.is_busy {
                    self.push_ui_error("Launch is not available while another backend operation is running");
                    return;
                }
                if self.ui.executable_path.trim().is_empty() {
                    self.push_ui_error("Set an executable path before launch");
                    return;
                }
                if !Path::new(self.ui.executable_path.trim()).exists() {
                    self.push_ui_error("Executable path does not exist");
                    return;
                }

                let target = TargetBinary {
                    program: self.ui.executable_path.trim().to_string(),
                    args: split_args(&self.ui.launch_args),
                    cwd: None,
                    env: Vec::new(),
                };
                self.remember_session(&target);
                self.reset_trace_for_new_session();
                self.clear_live_views();
                self.queue_backend_command(BackendCommand::Launch(target), "Launching target");
            }
            UiEvent::Attach => {
                if !self.ui.session.can_attach() {
                    self.push_ui_error("Attach is not available in the current session state");
                    return;
                }
                if let Ok(pid) = self.ui.attach_pid.trim().parse::<u32>() {
                    self.reset_trace_for_new_session();
                    self.clear_live_views();
                    self.queue_backend_command(BackendCommand::Attach(pid), format!("Attaching to pid {pid}"));
                } else {
                    self.push_ui_error("Attach PID is invalid");
                }
            }
            UiEvent::Continue => {
                if !self.ui.session.can_continue() {
                    self.push_ui_error("Continue is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::Continue, "Continuing execution");
            }
            UiEvent::Pause => {
                if !self.ui.session.can_pause() {
                    self.push_ui_error("Pause is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::Pause, "Pausing execution");
            }
            UiEvent::StepInto => {
                if !self.ui.session.can_step() {
                    self.push_ui_error("Step into is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::StepInto, "Stepping into instruction");
            }
            UiEvent::StepOver => {
                if !self.ui.session.can_step() {
                    self.push_ui_error("Step over is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::StepOver, "Stepping over instruction");
            }
            UiEvent::StepOut => {
                if !self.ui.session.can_step() {
                    self.push_ui_error("Step out is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::StepOut, "Stepping out of frame");
            }
            UiEvent::Restart => {
                if self.ui.session.is_busy || !self.ui.session.can_restart {
                    self.push_ui_error("Restart is not available in the current session state");
                    return;
                }
                if self.ui.executable_path.trim().is_empty() || !Path::new(self.ui.executable_path.trim()).exists() {
                    self.push_ui_error("Set a valid executable path before restart");
                    return;
                }
                let target = TargetBinary {
                    program: self.ui.executable_path.trim().to_string(),
                    args: split_args(&self.ui.launch_args),
                    cwd: None,
                    env: Vec::new(),
                };
                self.reset_trace_for_new_session();
                self.clear_live_views();
                self.queue_backend_command(BackendCommand::Restart(target), "Restarting target");
            }
            UiEvent::Stop => {
                if !self.ui.session.can_stop() {
                    self.push_ui_error("Stop is not available in the current session state");
                    return;
                }
                self.queue_backend_command(BackendCommand::Stop, "Stopping target");
            }
            UiEvent::Refresh => {
                if !self.ui.session.can_refresh() {
                    self.push_ui_error("Refresh is not available until a target is loaded");
                    return;
                }
                self.queue_backend_command(BackendCommand::Refresh, "Refreshing state");
            }
            UiEvent::ToggleBreakpoint(addr) => {
                if !self.ui.session.can_toggle_breakpoint() {
                    self.push_ui_error("Breakpoints are not available until a target is loaded");
                    return;
                }
                self.queue_backend_command(
                    BackendCommand::ToggleBreakpoint(addr),
                    format!("Toggling breakpoint at 0x{addr:x}"),
                );
            }
            UiEvent::RemoveBreakpoint(id) => {
                if !self.ui.session.can_toggle_breakpoint() {
                    self.push_ui_error("Breakpoints are not available until a target is loaded");
                    return;
                }
                self.queue_backend_command(
                    BackendCommand::RemoveBreakpoint(id),
                    format!("Removing breakpoint #{id}"),
                );
            }
            UiEvent::JumpMemory(addr) => {
                if !self.ui.session.can_jump_memory() {
                    self.push_ui_error("Memory inspection requires a live stopped process");
                    return;
                }
                self.ui.memory_base_input = format!("0x{addr:x}");
                self.queue_backend_command(
                    BackendCommand::JumpMemory(addr),
                    format!("Loading memory around 0x{addr:x}"),
                );
            }
            UiEvent::JumpDisassembly(addr) => {
                if !self.ui.session.can_refresh() {
                    self.push_ui_error("Disassembly navigation requires a loaded target");
                    return;
                }
                self.queue_backend_command(
                    BackendCommand::JumpDisassembly(addr),
                    format!("Jumping disassembly to 0x{addr:x}"),
                );
            }
            UiEvent::ClearOutput => self.ui.output_lines.clear(),
            UiEvent::ClearTrace => {
                self.reset_trace_for_new_session();
            }
            UiEvent::SetBottomTab(tab) => self.ui.bottom_tab = tab,
            UiEvent::JumpToCurrentInstruction => {
                if let Some(line) = self.ui.disassembly.iter().find(|line| line.is_current) {
                    self.ui.memory_base_input = format!("0x{:x}", line.address);
                    self.queue_backend_command(
                        BackendCommand::JumpMemory(line.address),
                        format!("Loading memory around 0x{:x}", line.address),
                    );
                }
            }
            UiEvent::ShowAbout => self.ui.output_lines.push(
                "tra86: Rust-native Assembly Tracer Analyzer (LLDB backend active)".to_string(),
            ),
            UiEvent::Quit => std::process::exit(0),
        }
    }

    fn apply_worker_message(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::Snapshot(snapshot) => {
                self.ui.status_line = snapshot.status_line;
                self.ui.session = snapshot.session;
                self.ui.session.is_busy = false;
                self.ui.stop_reason = snapshot.stop_reason;
                self.ui.threads = snapshot.threads;
                self.ui.frames = snapshot.frames;
                self.ui.disassembly = snapshot.disassembly;
                self.ui.function_rows = snapshot.function_rows;
                self.ui.breakpoints = snapshot.breakpoints;
                self.ui.memory_map_lines = snapshot.memory_map_lines;
                self.ui.memory_bytes = snapshot.memory_bytes;
                self.ui.output_lines.extend(snapshot.output_lines);
                self.ui.analysis_lines = build_analysis_lines(&self.ui.disassembly, &self.trace);
                if !self.ui.session.has_live_process {
                    self.ui.registers = None;
                    self.ui.register_diffs.clear();
                    self.previous_registers = None;
                    self.previous_sp = None;
                }
                let current_line = self
                    .ui
                    .disassembly
                    .iter()
                    .find(|line| line.is_current)
                    .cloned();

                if let Some(registers) = snapshot.registers {
                    let diff = compute_register_delta(&registers, self.previous_registers.as_ref());
                    self.ui.register_diffs = diff.clone();

                    let current_sp = parse_hex_register(&registers, "rsp").map(|v| v as i64);
                    let sp_delta = match (current_sp, self.previous_sp) {
                        (Some(current), Some(previous)) => current - previous,
                        _ => 0,
                    };

                    if let Some(current_line) = current_line.as_ref() {
                        if self.last_traced_ip != Some(current_line.address) {
                            let record = InstructionRecord {
                                address: current_line.address,
                                bytes: current_line.bytes.clone(),
                                mnemonic: current_line.mnemonic.clone(),
                                operands: current_line.operands.clone(),
                                symbol: current_line.function.as_ref().map(|name| SymbolInfo {
                                    name: name.clone(),
                                    module: None,
                                    offset: 0,
                                }),
                                source: current_line.source.clone(),
                            };

                            let delta =
                                build_delta(&current_line.mnemonic, diff, sp_delta, Vec::new());
                            let event = TraceEvent {
                                timestamp: Utc::now(),
                                thread_id: registers.thread_id,
                                instruction: record,
                                stop_reason: self.ui.stop_reason.clone(),
                                delta: Some(delta),
                            };
                            self.trace.push(event);
                            self.last_traced_ip = Some(current_line.address);
                            if self.trace.len() > 2_000 {
                                let drop_count = self.trace.len() - 2_000;
                                self.trace.drain(0..drop_count);
                            }
                            self.ui.trace = self.trace.clone();
                            self.ui.analysis_lines =
                                build_analysis_lines(&self.ui.disassembly, &self.trace);
                        }
                    }

                    self.previous_sp = current_sp;
                    self.previous_registers = Some(registers.clone());
                    self.ui.registers = Some(registers);
                } else if let Some(current_line) = current_line.as_ref() {
                    if self.last_traced_ip != Some(current_line.address) {
                        let record = InstructionRecord {
                            address: current_line.address,
                            bytes: current_line.bytes.clone(),
                            mnemonic: current_line.mnemonic.clone(),
                            operands: current_line.operands.clone(),
                            symbol: current_line.function.as_ref().map(|name| SymbolInfo {
                                name: name.clone(),
                                module: None,
                                offset: 0,
                            }),
                            source: current_line.source.clone(),
                        };
                        self.trace.push(TraceEvent {
                            timestamp: Utc::now(),
                            thread_id: self.ui.threads.first().map(|t| t.id).unwrap_or(1),
                            instruction: record,
                            stop_reason: self.ui.stop_reason.clone(),
                            delta: None,
                        });
                        self.last_traced_ip = Some(current_line.address);
                        if self.trace.len() > 2_000 {
                            let drop_count = self.trace.len() - 2_000;
                            self.trace.drain(0..drop_count);
                        }
                        self.ui.trace = self.trace.clone();
                        self.ui.analysis_lines =
                            build_analysis_lines(&self.ui.disassembly, &self.trace);
                    }
                }
            }
            WorkerMessage::Error(error) => {
                self.ui.session.is_busy = false;
                self.ui.session.last_error = Some(error.clone());
                self.ui.output_lines.push(format!("error: {error}"));
            }
        }
    }

    fn remember_session(&mut self, target: &TargetBinary) {
        if target.program.is_empty() {
            return;
        }

        let entry = RecentSession {
            program: target.program.clone(),
            args: target.args.clone(),
            timestamp_utc: Utc::now().to_rfc3339(),
        };

        self.recents
            .items
            .retain(|existing| existing.program != entry.program);
        self.recents.items.insert(0, entry);
        if self.recents.items.len() > 20 {
            self.recents.items.truncate(20);
        }
        self.recents.save();
    }
}

fn split_args(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .map(|part| part.to_string())
        .collect()
}

fn parse_hex_register(registers: &RegisterBank, name: &str) -> Option<u64> {
    let reg = registers.registers.iter().find(|r| r.name == name)?;
    let value = reg.value.strip_prefix("0x").unwrap_or(reg.value.as_str());
    u64::from_str_radix(value, 16).ok()
}

fn build_analysis_lines(disassembly: &[DisassemblyLine], trace: &[TraceEvent]) -> Vec<String> {
    use std::collections::BTreeMap;

    let mut lines = Vec::new();
    lines.push(format!("Trace events: {}", trace.len()));
    lines.push(format!("Disassembly rows loaded: {}", disassembly.len()));

    let mut mnemonic_counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in disassembly {
        *mnemonic_counts.entry(row.mnemonic.clone()).or_default() += 1;
    }

    let mut mnemonics_sorted = mnemonic_counts.into_iter().collect::<Vec<_>>();
    mnemonics_sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    lines.push("Top mnemonics:".to_string());
    for (mnemonic, count) in mnemonics_sorted.into_iter().take(12) {
        lines.push(format!("  {:>8}  {}", mnemonic, count));
    }

    let mut branch_count = 0usize;
    let mut call_count = 0usize;
    let mut ret_count = 0usize;
    let mut mem_access_count = 0usize;
    for row in disassembly {
        let mn = row.mnemonic.to_ascii_lowercase();
        if mn.starts_with('j') {
            branch_count += 1;
        }
        if mn == "call" || mn == "bl" {
            call_count += 1;
        }
        if mn == "ret" {
            ret_count += 1;
        }
        if row.operands.contains('[') || row.operands.contains("ptr") {
            mem_access_count += 1;
        }
    }
    lines.push(format!(
        "Control-flow: branches={} calls={} returns={}",
        branch_count, call_count, ret_count
    ));
    lines.push(format!(
        "Memory-access-looking instructions: {}",
        mem_access_count
    ));

    let mut changed_register_hist: BTreeMap<String, usize> = BTreeMap::new();
    for event in trace.iter().rev().take(500) {
        if let Some(delta) = &event.delta {
            for reg in &delta.changed_registers {
                *changed_register_hist.entry(reg.name.clone()).or_default() += 1;
            }
        }
    }
    let mut reg_sorted = changed_register_hist.into_iter().collect::<Vec<_>>();
    reg_sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    lines.push("Changed-register heat (recent trace):".to_string());
    for (reg, count) in reg_sorted.into_iter().take(12) {
        lines.push(format!("  {:>8}  {}", reg, count));
    }

    lines
}

fn build_function_rows(disassembly: &[DisassemblyLine]) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut functions: BTreeMap<String, u64> = BTreeMap::new();
    for line in disassembly {
        if let Some(name) = &line.function {
            functions.entry(name.clone()).or_insert(line.address);
        }
    }

    if functions.is_empty() {
        disassembly
            .iter()
            .take(32)
            .map(|line| format!("0x{:016x} {}", line.address, line.mnemonic))
            .collect()
    } else {
        functions
            .into_iter()
            .map(|(name, addr)| format!("0x{addr:016x} {name}"))
            .collect()
    }
}

struct BackendWorker {
    tx: Sender<BackendCommand>,
    rx: Receiver<WorkerMessage>,
}

impl BackendWorker {
    fn spawn(initial: BackendChoice) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        // Backend calls run off the UI thread so disassembly/memory operations
        // do not block egui frame updates.
        thread::spawn(move || {
            let mut runtime = WorkerRuntime::new(initial);
            while let Ok(command) = command_rx.recv() {
                let result = runtime.handle_command(command);
                match result {
                    Ok(snapshot) => {
                        let _ = event_tx.send(WorkerMessage::Snapshot(snapshot));
                    }
                    Err(err) => {
                        let _ = event_tx.send(WorkerMessage::Error(err.to_string()));
                    }
                }
            }
        });

        Self {
            tx: command_tx,
            rx: event_rx,
        }
    }

    fn send(&self, cmd: BackendCommand) {
        let _ = self.tx.send(cmd);
    }

    fn try_recv(&self) -> Option<WorkerMessage> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(WorkerMessage::Error(
                "backend worker disconnected".to_string(),
            )),
        }
    }
}

#[derive(Debug)]
enum BackendCommand {
    SwitchBackend(BackendChoice),
    OpenTarget(String),
    Launch(TargetBinary),
    Attach(u32),
    Continue,
    Pause,
    StepInto,
    StepOver,
    StepOut,
    Restart(TargetBinary),
    Stop,
    Refresh,
    ToggleBreakpoint(u64),
    RemoveBreakpoint(u64),
    JumpMemory(u64),
    JumpDisassembly(u64),
}

#[derive(Debug)]
enum WorkerMessage {
    Snapshot(BackendSnapshot),
    Error(String),
}

#[derive(Debug)]
struct BackendSnapshot {
    status_line: String,
    session: SessionStatus,
    stop_reason: StopReason,
    threads: Vec<tra86_core::ThreadState>,
    frames: Vec<tra86_core::FrameState>,
    disassembly: Vec<DisassemblyLine>,
    function_rows: Vec<String>,
    registers: Option<RegisterBank>,
    breakpoints: Vec<Breakpoint>,
    memory_map_lines: Vec<String>,
    memory_bytes: Vec<u8>,
    output_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimePhase {
    Idle,
    TargetLoaded,
    Stopped,
    Exited,
    Detached,
}

#[derive(Debug, Clone)]
struct RuntimeSession {
    phase: RuntimePhase,
    target_program: Option<String>,
    attached_pid: Option<u32>,
    has_live_process: bool,
    last_error: Option<String>,
}

impl Default for RuntimeSession {
    fn default() -> Self {
        Self {
            phase: RuntimePhase::Idle,
            target_program: None,
            attached_pid: None,
            has_live_process: false,
            last_error: None,
        }
    }
}

impl RuntimeSession {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn has_target(&self) -> bool {
        self.target_program.is_some() || self.attached_pid.is_some()
    }

    fn can_restart(&self) -> bool {
        self.target_program.is_some()
    }

    fn set_target_program(&mut self, program: String) {
        self.target_program = Some(program);
        self.attached_pid = None;
        self.has_live_process = false;
        self.phase = RuntimePhase::TargetLoaded;
        self.last_error = None;
    }

    fn set_attached_pid(&mut self, pid: u32) {
        self.attached_pid = Some(pid);
        self.target_program = None;
        self.has_live_process = true;
        self.phase = RuntimePhase::Stopped;
        self.last_error = None;
    }

    fn mark_process_stopped(&mut self) {
        self.has_live_process = true;
        self.phase = RuntimePhase::Stopped;
        self.last_error = None;
    }

    fn mark_exited(&mut self) {
        self.has_live_process = false;
        self.attached_pid = None;
        self.phase = RuntimePhase::Exited;
    }

    fn mark_detached(&mut self) {
        self.has_live_process = false;
        self.attached_pid = None;
        self.phase = RuntimePhase::Detached;
    }

    fn note_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
    }

    fn reconcile_from_snapshot(&mut self, stop_reason: &StopReason, has_live_process: bool) {
        match stop_reason {
            StopReason::Exited(_) => self.mark_exited(),
            StopReason::Detached => self.mark_detached(),
            _ if has_live_process => self.mark_process_stopped(),
            _ if self.has_target() => {
                self.has_live_process = false;
                if matches!(self.phase, RuntimePhase::Stopped) {
                    self.phase = RuntimePhase::TargetLoaded;
                }
            }
            _ => self.phase = RuntimePhase::Idle,
        }
    }

    fn to_ui_status(&self, is_busy: bool) -> SessionStatus {
        let detail = if is_busy {
            self.last_error
                .clone()
                .unwrap_or_else(|| "Working...".to_string())
        } else {
            match self.phase {
                RuntimePhase::Idle => "No target loaded".to_string(),
                RuntimePhase::TargetLoaded => self
                    .target_program
                    .as_ref()
                    .map(|program| format!("Target loaded: {program}"))
                    .unwrap_or_else(|| "Target loaded".to_string()),
                RuntimePhase::Stopped => {
                    if let Some(pid) = self.attached_pid {
                        format!("Attached to pid {pid} and stopped")
                    } else if let Some(program) = &self.target_program {
                        format!("Stopped in {program}")
                    } else {
                        "Stopped".to_string()
                    }
                }
                RuntimePhase::Exited => "Process exited".to_string(),
                RuntimePhase::Detached => "Detached from process".to_string(),
            }
        };

        SessionStatus {
            phase: match self.phase {
                RuntimePhase::Idle => SessionPhase::Idle,
                RuntimePhase::TargetLoaded => SessionPhase::TargetLoaded,
                RuntimePhase::Stopped => SessionPhase::Stopped,
                RuntimePhase::Exited => SessionPhase::Exited,
                RuntimePhase::Detached => SessionPhase::Detached,
            },
            detail,
            is_busy,
            has_target: self.has_target(),
            has_live_process: self.has_live_process,
            can_restart: self.can_restart(),
            last_error: self.last_error.clone(),
        }
    }
}

struct WorkerRuntime {
    orchestrator: BackendOrchestrator,
    backend_choice: BackendChoice,
    session: RuntimeSession,
    breakpoints: Vec<Breakpoint>,
    memory_address: u64,
    disassembly_address: Option<u64>,
    output_lines: Vec<String>,
}

impl WorkerRuntime {
    fn new(initial: BackendChoice) -> Self {
        Self {
            orchestrator: BackendOrchestrator::new(make_backend(initial)),
            backend_choice: initial,
            session: RuntimeSession::default(),
            breakpoints: Vec::new(),
            memory_address: 0x1000,
            disassembly_address: None,
            output_lines: vec![format!("backend initialized: {initial:?}")],
        }
    }

    fn handle_command(&mut self, command: BackendCommand) -> anyhow::Result<BackendSnapshot> {
        self.session.last_error = None;
        if let Err(err) = self.apply_command(command) {
            let message = err.to_string();
            self.output_lines.push(format!("command failed: {message}"));
            self.session.note_error(message);
        }

        self.collect_snapshot()
    }

    fn apply_command(&mut self, command: BackendCommand) -> anyhow::Result<()> {
        match command {
            BackendCommand::SwitchBackend(choice) => {
                self.backend_choice = choice;
                self.orchestrator.replace_backend(make_backend(choice))?;
                self.session.reset();
                self.breakpoints.clear();
                self.memory_address = 0x1000;
                self.disassembly_address = None;
                self.output_lines
                    .push(format!("switched backend to {:?}", self.backend_choice));
            }
            BackendCommand::OpenTarget(program) => {
                self.orchestrator.backend_mut().open_target(&program)?;
                self.session.set_target_program(program.clone());
                self.output_lines.push(format!("loaded target {}", program));
            }
            BackendCommand::Launch(target) => {
                self.orchestrator.backend_mut().launch(LaunchRequest {
                    target: target.clone(),
                })?;
                self.session.set_target_program(target.program.clone());
                self.session.mark_process_stopped();
                self.output_lines
                    .push(format!("launched {}", target.program));
            }
            BackendCommand::Attach(pid) => {
                self.orchestrator.backend_mut().attach(pid)?;
                self.session.set_attached_pid(pid);
                self.output_lines.push(format!("attached to pid {pid}"));
            }
            BackendCommand::Continue => {
                self.orchestrator.backend_mut().continue_exec()?;
            }
            BackendCommand::Pause => {
                self.orchestrator.backend_mut().pause()?;
            }
            BackendCommand::StepInto => {
                self.orchestrator.backend_mut().step_into()?;
            }
            BackendCommand::StepOver => {
                self.orchestrator.backend_mut().step_over()?;
            }
            BackendCommand::StepOut => {
                self.orchestrator.backend_mut().step_out()?;
            }
            BackendCommand::Restart(target) => {
                let backend = self.orchestrator.backend_mut();
                let _ = backend.kill();
                backend.launch(LaunchRequest {
                    target: target.clone(),
                })?;
                self.session.set_target_program(target.program.clone());
                self.session.mark_process_stopped();
                self.output_lines
                    .push(format!("restarted {}", target.program));
            }
            BackendCommand::Stop => {
                self.orchestrator.backend_mut().kill()?;
                self.session.mark_exited();
            }
            BackendCommand::Refresh => {}
            BackendCommand::ToggleBreakpoint(address) => {
                if let Some(existing_id) = self.breakpoints.iter().find_map(|bp| {
                    matches!(bp.location, BreakpointLocation::Address(addr) if addr == address)
                        .then_some(bp.id)
                }) {
                    self.orchestrator
                        .backend_mut()
                        .remove_breakpoint(existing_id)?;
                    self.breakpoints.retain(|bp| bp.id != existing_id);
                } else {
                    let bp = self
                        .orchestrator
                        .backend_mut()
                        .set_breakpoint(BreakpointLocation::Address(address))?;
                    self.breakpoints.push(bp);
                }
            }
            BackendCommand::RemoveBreakpoint(id) => {
                self.orchestrator.backend_mut().remove_breakpoint(id)?;
                self.breakpoints.retain(|bp| bp.id != id);
            }
            BackendCommand::JumpMemory(address) => {
                self.memory_address = address;
            }
            BackendCommand::JumpDisassembly(address) => {
                self.disassembly_address = Some(address);
            }
        }

        Ok(())
    }

    fn collect_snapshot(&mut self) -> anyhow::Result<BackendSnapshot> {
        let backend_name = self.orchestrator.backend_name().to_string();

        if !self.session.has_target() {
            let output_lines = std::mem::take(&mut self.output_lines);
            return Ok(BackendSnapshot {
                status_line: format!("backend={backend_name} idle"),
                session: self.session.to_ui_status(false),
                stop_reason: StopReason::None,
                threads: Vec::new(),
                frames: Vec::new(),
                disassembly: Vec::new(),
                function_rows: Vec::new(),
                registers: None,
                breakpoints: self.breakpoints.clone(),
                memory_map_lines: Vec::new(),
                memory_bytes: Vec::new(),
                output_lines,
            });
        }

        if !self.session.has_live_process && self.session.target_program.is_some() {
            let mut disassembly = match self.orchestrator.backend_mut().disassemble(
                self.disassembly_address,
                if self.disassembly_address.is_some() { 192 } else { 4_096 },
            ) {
                Ok(disassembly) => disassembly,
                Err(err) => {
                    self.output_lines
                        .push(format!("snapshot: failed to disassemble loaded target: {err}"));
                    Vec::new()
                }
            };

            let function_rows = build_function_rows(&disassembly);
            for line in &mut disassembly {
                line.is_current = false;
            }
            let stop_reason = match self.session.phase {
                RuntimePhase::Exited => StopReason::Exited(0),
                RuntimePhase::Detached => StopReason::Detached,
                _ => StopReason::None,
            };
            let output_lines = std::mem::take(&mut self.output_lines);

            return Ok(BackendSnapshot {
                status_line: format!("backend={backend_name} target_loaded"),
                session: self.session.to_ui_status(false),
                stop_reason,
                threads: Vec::new(),
                frames: Vec::new(),
                disassembly,
                function_rows,
                registers: None,
                breakpoints: self.breakpoints.clone(),
                memory_map_lines: Vec::new(),
                memory_bytes: Vec::new(),
                output_lines,
            });
        }

        let stop_reason = match self.orchestrator.backend_mut().current_stop_reason() {
            Ok(reason) => reason,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to read stop reason: {err}"));
                StopReason::None
            }
        };

        let threads = match self.orchestrator.backend_mut().list_threads() {
            Ok(threads) => threads,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to list threads: {err}"));
                Vec::new()
            }
        };
        let current_thread_id = threads.first().map(|t| t.id).unwrap_or(1);
        let frames = match self.orchestrator.backend_mut().list_frames(current_thread_id) {
            Ok(frames) => frames,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to list frames: {err}"));
                Vec::new()
            }
        };

        let registers = match self.orchestrator.backend_mut().read_registers(current_thread_id) {
            Ok(registers) => Some(registers),
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to read registers: {err}"));
                None
            }
        };
        let mut current_ip = match self
            .orchestrator
            .backend_mut()
            .current_instruction(current_thread_id)
        {
            Ok(ip) => ip,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to read current instruction: {err}"));
                None
            }
        };

        let has_live_process = !threads.is_empty() || registers.is_some() || current_ip.is_some();
        self.session
            .reconcile_from_snapshot(&stop_reason, has_live_process);

        let disassembly_anchor = self.disassembly_address.or(current_ip);
        let disassembly_count = if disassembly_anchor.is_some() {
            192
        } else {
            4_096
        };
        let mut disassembly = match self
            .orchestrator
            .backend_mut()
            .disassemble(disassembly_anchor, disassembly_count)
        {
            Ok(disassembly) => disassembly,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to disassemble: {err}"));
                Vec::new()
            }
        };
        if current_ip.is_none() {
            current_ip = disassembly
                .iter()
                .find(|line| line.is_current)
                .map(|line| line.address)
                .or_else(|| disassembly.first().map(|line| line.address));
        }
        if let Some(ip) = current_ip {
            for line in &mut disassembly {
                line.is_current = line.address == ip;
            }
        }
        let program_tree_disassembly = match self.orchestrator.backend_mut().disassemble(None, 4_096)
        {
            Ok(disassembly) => disassembly,
            Err(err) => {
                self.output_lines.push(format!(
                    "snapshot: failed to load full program disassembly: {err}"
                ));
                disassembly.clone()
            }
        };
        let function_rows = build_function_rows(&program_tree_disassembly);

        let memory_map = match self.orchestrator.backend_mut().memory_map() {
            Ok(memory_map) => memory_map,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to read memory map: {err}"));
                Vec::new()
            }
        };
        let memory_map_lines = memory_map
            .iter()
            .map(|region| {
                format!(
                    "0x{:016x}-0x{:016x} {}{}{} {}",
                    region.start,
                    region.end,
                    if region.permissions.read { 'r' } else { '-' },
                    if region.permissions.write { 'w' } else { '-' },
                    if region.permissions.execute { 'x' } else { '-' },
                    region
                        .label
                        .clone()
                        .or_else(|| region.pathname.clone())
                        .unwrap_or_else(|| "anonymous".to_string())
                )
            })
            .collect::<Vec<_>>();

        if self.memory_address == 0x1000 {
            if let Some(ip) = current_ip {
                self.memory_address = ip;
            }
        }

        let memory_bytes = match self
            .orchestrator
            .backend_mut()
            .read_memory(self.memory_address, 0x200)
        {
            Ok(memory) => memory,
            Err(err) => {
                self.output_lines
                    .push(format!("snapshot: failed to read memory: {err}"));
                Vec::new()
            }
        };

        let output_lines = std::mem::take(&mut self.output_lines);

        Ok(BackendSnapshot {
            status_line: format!("backend={backend_name} thread_count={}", threads.len()),
            session: self.session.to_ui_status(false),
            stop_reason,
            threads,
            frames,
            disassembly,
            function_rows,
            registers,
            breakpoints: self.breakpoints.clone(),
            memory_map_lines,
            memory_bytes,
            output_lines,
        })
    }
}

fn make_backend(choice: BackendChoice) -> Box<dyn DebugBackend> {
    match choice {
        BackendChoice::Mock => Box::new(MockBackend::default()),
        BackendChoice::Lldb => Box::new(LldbBackend::new()),
    }
}

const RECENTS_PATH: &str = "recent_sessions.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RecentSessions {
    items: Vec<RecentSession>,
}

impl RecentSessions {
    fn load() -> Self {
        let path = Path::new(RECENTS_PATH);
        if !path.exists() {
            return Self::default();
        }

        let content = fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(serialized) = serde_json::to_string_pretty(self) {
            let _ = fs::write(Path::new(RECENTS_PATH), serialized);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentSession {
    program: String,
    args: Vec<String>,
    timestamp_utc: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_without_target_stays_idle_and_quiet() {
        let mut runtime = WorkerRuntime::new(BackendChoice::Mock);
        let snapshot = runtime
            .handle_command(BackendCommand::Refresh)
            .expect("refresh should succeed");

        assert_eq!(snapshot.session.phase, SessionPhase::Idle);
        assert!(!snapshot.session.has_target);
        assert!(snapshot.output_lines.iter().all(|line| !line.contains("failed")));
        assert!(snapshot.disassembly.is_empty());
    }

    #[test]
    fn open_target_creates_non_live_loaded_state() {
        let mut runtime = WorkerRuntime::new(BackendChoice::Mock);
        let snapshot = runtime
            .handle_command(BackendCommand::OpenTarget("/tmp/mock-target".to_string()))
            .expect("open target should succeed");

        assert_eq!(snapshot.session.phase, SessionPhase::TargetLoaded);
        assert!(snapshot.session.has_target);
        assert!(!snapshot.session.has_live_process);
        assert!(!snapshot.disassembly.is_empty());
        assert!(snapshot.threads.is_empty());
    }

    #[test]
    fn launch_and_stop_transition_through_stopped_and_exited() {
        let mut runtime = WorkerRuntime::new(BackendChoice::Mock);
        let target = TargetBinary {
            program: "/tmp/mock-target".to_string(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        };

        let launched = runtime
            .handle_command(BackendCommand::Launch(target.clone()))
            .expect("launch should succeed");
        assert_eq!(launched.session.phase, SessionPhase::Stopped);
        assert!(launched.session.has_live_process);
        assert!(!launched.threads.is_empty());

        let stopped = runtime
            .handle_command(BackendCommand::Stop)
            .expect("stop should succeed");
        assert_eq!(stopped.session.phase, SessionPhase::Exited);
        assert!(!stopped.session.has_live_process);
        assert!(stopped.threads.is_empty());
        assert!(matches!(stopped.stop_reason, StopReason::Exited(_)));
        assert!(!stopped.disassembly.is_empty());
    }
}
