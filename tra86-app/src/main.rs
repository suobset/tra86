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
use tra86_ui::{BackendChoice, UiEvent, UiModel};

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
    fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::SetBackend(choice) => {
                self.ui.backend_choice = choice;
                self.worker.send(BackendCommand::SwitchBackend(choice));
                if !self.ui.executable_path.trim().is_empty() {
                    self.worker
                        .send(BackendCommand::OpenTarget(self.ui.executable_path.clone()));
                }
                self.trace.clear();
                self.ui.trace.clear();
                self.ui.frames.clear();
                self.last_traced_ip = None;
                self.ui.status_line = format!("Switched backend to {choice:?}");
            }
            UiEvent::OpenExecutablePicker => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.ui.executable_path = path.display().to_string();
                    self.ui
                        .output_lines
                        .push(format!("Configured target: {}", self.ui.executable_path));
                    self.worker
                        .send(BackendCommand::OpenTarget(self.ui.executable_path.clone()));
                }
            }
            UiEvent::Launch => {
                if self.ui.executable_path.trim().is_empty() {
                    self.ui
                        .output_lines
                        .push("Set an executable path before launch".to_string());
                    return;
                }
                if self.ui.backend_choice == BackendChoice::Mock
                    && !self.ui.executable_path.trim().is_empty()
                {
                    self.ui.backend_choice = BackendChoice::Lldb;
                    self.worker
                        .send(BackendCommand::SwitchBackend(BackendChoice::Lldb));
                    self.ui
                        .output_lines
                        .push("Auto-switched backend to LLDB for executable launch".to_string());
                }

                let target = TargetBinary {
                    program: self.ui.executable_path.trim().to_string(),
                    args: split_args(&self.ui.launch_args),
                    cwd: None,
                    env: Vec::new(),
                };
                self.remember_session(&target);
                self.worker
                    .send(BackendCommand::OpenTarget(target.program.clone()));
                self.worker.send(BackendCommand::Launch(target));
            }
            UiEvent::Attach => {
                if let Ok(pid) = self.ui.attach_pid.trim().parse::<u32>() {
                    self.worker.send(BackendCommand::Attach(pid));
                } else {
                    self.ui
                        .output_lines
                        .push("Attach PID is invalid".to_string());
                }
            }
            UiEvent::Continue => self.worker.send(BackendCommand::Continue),
            UiEvent::Pause => self.worker.send(BackendCommand::Pause),
            UiEvent::StepInto => self.worker.send(BackendCommand::StepInto),
            UiEvent::StepOver => self.worker.send(BackendCommand::StepOver),
            UiEvent::StepOut => self.worker.send(BackendCommand::StepOut),
            UiEvent::Restart => {
                let target = TargetBinary {
                    program: self.ui.executable_path.trim().to_string(),
                    args: split_args(&self.ui.launch_args),
                    cwd: None,
                    env: Vec::new(),
                };
                self.worker.send(BackendCommand::Restart(target));
            }
            UiEvent::Stop => self.worker.send(BackendCommand::Stop),
            UiEvent::Refresh => self.worker.send(BackendCommand::Refresh),
            UiEvent::ToggleBreakpoint(addr) => {
                self.worker.send(BackendCommand::ToggleBreakpoint(addr))
            }
            UiEvent::RemoveBreakpoint(id) => self.worker.send(BackendCommand::RemoveBreakpoint(id)),
            UiEvent::JumpMemory(addr) => {
                self.ui.memory_base_input = format!("0x{addr:x}");
                self.worker.send(BackendCommand::JumpMemory(addr));
            }
            UiEvent::JumpDisassembly(addr) => {
                self.worker.send(BackendCommand::JumpDisassembly(addr));
            }
            UiEvent::ClearOutput => self.ui.output_lines.clear(),
            UiEvent::ClearTrace => {
                self.trace.clear();
                self.ui.trace.clear();
            }
            UiEvent::SetBottomTab(tab) => self.ui.bottom_tab = tab,
            UiEvent::JumpToCurrentInstruction => {
                if let Some(line) = self.ui.disassembly.iter().find(|line| line.is_current) {
                    self.ui.memory_base_input = format!("0x{:x}", line.address);
                    self.worker.send(BackendCommand::JumpMemory(line.address));
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

struct WorkerRuntime {
    orchestrator: BackendOrchestrator,
    backend_choice: BackendChoice,
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
            breakpoints: Vec::new(),
            memory_address: 0x1000,
            disassembly_address: None,
            output_lines: vec![format!("backend initialized: {initial:?}")],
        }
    }

    fn handle_command(&mut self, command: BackendCommand) -> anyhow::Result<BackendSnapshot> {
        match command {
            BackendCommand::SwitchBackend(choice) => {
                self.backend_choice = choice;
                self.orchestrator.replace_backend(make_backend(choice))?;
                self.breakpoints.clear();
                self.output_lines
                    .push(format!("switched backend to {:?}", self.backend_choice));
            }
            BackendCommand::OpenTarget(program) => {
                self.orchestrator.backend_mut().open_target(&program)?;
                self.output_lines.push(format!("loaded target {}", program));
            }
            BackendCommand::Launch(target) => {
                self.orchestrator.backend_mut().launch(LaunchRequest {
                    target: target.clone(),
                })?;
                self.output_lines
                    .push(format!("launched {}", target.program));
            }
            BackendCommand::Attach(pid) => {
                self.orchestrator.backend_mut().attach(pid)?;
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
                self.output_lines
                    .push(format!("restarted {}", target.program));
            }
            BackendCommand::Stop => {
                self.orchestrator.backend_mut().kill()?;
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

        self.collect_snapshot()
    }

    fn collect_snapshot(&mut self) -> anyhow::Result<BackendSnapshot> {
        let backend_name = self.orchestrator.backend_name().to_string();

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
