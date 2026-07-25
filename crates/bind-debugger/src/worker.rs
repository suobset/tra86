//! The debugger worker thread and its handle.
//!
//! The worker owns the [`DebugBackend`] and is the *only* thing that touches
//! it. The application thread talks to the worker over two bounded-ish mpsc
//! channels: [`Command`]s go down, [`WorkerUpdate`]s come up. This guarantees
//! the debugger never runs on the render thread and that state transitions are
//! driven by the event stream rather than races between UI ticks and backend
//! callbacks.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bind_core::{
    Address, BindError, Command, DebugEvent, EventSeq, ProcessLifecycle, RegisterChange,
    RegisterSet, SequencedEvent, SessionId, SessionSnapshot, StepKind, ThreadId,
};

use crate::backend::DebugBackend;

/// Messages the worker sends up to the application thread.
#[derive(Debug, Clone)]
pub enum WorkerUpdate {
    /// A normalized, sequenced event for the timeline / trace / analyses.
    Event(SequencedEvent),
    /// A fresh consistent read model. Produced on state changes, not per frame.
    Snapshot(Box<SessionSnapshot>),
    /// Result of a `ReadMemory` command.
    Memory { addr: Address, bytes: Vec<u8> },
    /// A command failed; surfaced explicitly rather than swallowed.
    CommandError(BindError),
    /// The worker thread has stopped and will send nothing further.
    Shutdown,
}

/// The application-side handle to the worker.
pub struct WorkerHandle {
    commands: Sender<Command>,
    updates: Receiver<WorkerUpdate>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    /// Spawns a worker owning `backend` for session `id`.
    pub fn spawn(id: SessionId, backend: Box<dyn DebugBackend>) -> WorkerHandle {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let (upd_tx, upd_rx) = std::sync::mpsc::channel::<WorkerUpdate>();
        let join = thread::Builder::new()
            .name("bind-debugger".into())
            .spawn(move || {
                let mut worker = Worker::new(id, backend, upd_tx);
                worker.run(cmd_rx);
            })
            .expect("failed to spawn debugger worker");
        WorkerHandle {
            commands: cmd_tx,
            updates: upd_rx,
            join: Some(join),
        }
    }

    /// Queues a command. Returns an error only if the worker has gone away.
    pub fn send(&self, command: Command) -> Result<(), BindError> {
        self.commands
            .send(command)
            .map_err(|_| BindError::Internal("debugger worker disconnected".into()))
    }

    /// Non-blocking drain of pending updates.
    pub fn drain(&self) -> Vec<WorkerUpdate> {
        let mut out = Vec::new();
        while let Ok(u) = self.updates.try_recv() {
            out.push(u);
        }
        out
    }

    /// Requests shutdown and joins the worker thread.
    pub fn shutdown(mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Idle poll interval: how often the worker checks a running backend for
/// asynchronously-arriving events when no command is pending.
const IDLE_POLL: Duration = Duration::from_millis(20);

/// How many instructions of disassembly to fetch around the pc on each stop.
const DISASM_WINDOW: usize = 24;

struct Worker {
    id: SessionId,
    backend: Box<dyn DebugBackend>,
    updates: Sender<WorkerUpdate>,
    seq: EventSeq,
    start: Instant,
    prev_registers: Option<RegisterSet>,
    selected_thread: Option<ThreadId>,
    /// The program under debug, remembered from the attach request so every
    /// snapshot can carry it (backends don't expose it through the trait).
    program: Option<String>,
}

impl Worker {
    fn new(id: SessionId, backend: Box<dyn DebugBackend>, updates: Sender<WorkerUpdate>) -> Self {
        Self {
            id,
            backend,
            updates,
            seq: EventSeq::ZERO,
            start: Instant::now(),
            prev_registers: None,
            selected_thread: None,
            program: None,
        }
    }

    fn run(&mut self, commands: Receiver<Command>) {
        loop {
            match commands.recv_timeout(IDLE_POLL) {
                Ok(Command::Shutdown) => break,
                Ok(command) => {
                    if self.handle_command(command) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Idle: pick up any async backend notifications.
                    self.pump_events();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = self.updates.send(WorkerUpdate::Shutdown);
    }

    /// Returns true if the worker should exit.
    fn handle_command(&mut self, command: Command) -> bool {
        let result: Result<bool, BindError> = (|| {
            match command {
                Command::Shutdown => return Ok(true),
                Command::Attach(spec) => {
                    self.program = match &spec {
                        bind_core::AttachSpec::Launch(t) => Some(t.program.clone()),
                        bind_core::AttachSpec::CoreFile { program, .. } => Some(program.clone()),
                        bind_core::AttachSpec::Pid(pid) => Some(format!("pid {pid}")),
                    };
                    self.backend.attach(&spec)?
                }
                Command::Continue => self.backend.resume()?,
                Command::Pause => self.backend.pause()?,
                Command::Step(kind) => self.backend.step(kind)?,
                Command::RunTo(target) => self.backend.run_to(&target)?,
                Command::Detach => self.backend.detach()?,
                Command::Terminate => self.backend.terminate()?,
                Command::SelectThread(t) => {
                    self.selected_thread = Some(t);
                    self.backend.select_thread(t)?;
                }
                Command::SelectFrame(f) => self.backend.select_frame(f.raw() as usize)?,
                Command::AddBreakpoint {
                    location,
                    condition,
                } => {
                    self.backend
                        .add_breakpoint(&location, condition.as_deref())?;
                }
                Command::RemoveBreakpoint(id) => self.backend.remove_breakpoint(id)?,
                Command::EnableBreakpoint(id, on) => self.backend.enable_breakpoint(id, on)?,
                Command::ReadRegisters(_) => { /* handled by refresh below */ }
                Command::ReadMemory { addr, len } => {
                    let bytes = self.backend.read_memory(addr, len)?;
                    let _ = self.updates.send(WorkerUpdate::Memory { addr, bytes });
                }
                Command::Disassemble { .. } => { /* refresh below re-fetches */ }
                Command::TraceStart { .. } | Command::TraceStop => {
                    // Trace lifecycle is owned by the application layer; the
                    // worker simply keeps emitting events.
                }
            }
            Ok(false)
        })();

        match result {
            Ok(should_exit) => {
                self.pump_events();
                should_exit
            }
            Err(err) => {
                let _ = self.updates.send(WorkerUpdate::CommandError(err));
                // Still pump: a partially-applied command may have produced
                // events we must not drop.
                self.pump_events();
                false
            }
        }
    }

    /// Drains backend events, forwards them sequenced, and if anything changed
    /// the observable state, publishes a fresh snapshot.
    fn pump_events(&mut self) {
        let events = self.backend.poll_events();
        if events.is_empty() {
            return;
        }
        let mut needs_snapshot = false;
        for event in &events {
            match event {
                DebugEvent::Stopped { thread, .. } => {
                    self.selected_thread = Some(*thread);
                    needs_snapshot = true;
                }
                DebugEvent::ProcessExited { .. }
                | DebugEvent::ProcessDetached
                | DebugEvent::TargetLoaded { .. }
                | DebugEvent::ModuleLoaded { .. }
                | DebugEvent::JitCodeLoaded { .. }
                | DebugEvent::BreakpointResolved { .. } => needs_snapshot = true,
                _ => {}
            }
        }
        for event in events {
            self.seq = self.seq.next();
            let mono = self.start.elapsed().as_nanos() as u64;
            let sequenced = SequencedEvent::new(self.seq, mono, event);
            let _ = self.updates.send(WorkerUpdate::Event(sequenced));
        }
        if needs_snapshot {
            self.publish_snapshot();
        }
    }

    fn publish_snapshot(&mut self) {
        match self.build_snapshot() {
            Ok(snap) => {
                let _ = self.updates.send(WorkerUpdate::Snapshot(Box::new(snap)));
            }
            Err(err) => {
                let _ = self.updates.send(WorkerUpdate::CommandError(err));
            }
        }
    }

    fn build_snapshot(&mut self) -> Result<SessionSnapshot, BindError> {
        let caps = self.backend.capabilities();
        let name = self.backend.name().to_string();
        let mut snap = SessionSnapshot::empty(self.id, name, caps);
        snap.program.clone_from(&self.program);

        let process = self.backend.process_info()?;
        let stopped = process.lifecycle.is_stopped();
        snap.process = process;
        snap.modules = self.backend.modules().unwrap_or_default();
        snap.memory_regions = self.backend.memory_regions().unwrap_or_default();
        snap.breakpoints = self.backend.breakpoints().unwrap_or_default();

        if stopped {
            snap.threads = self.backend.threads().unwrap_or_default();
            let thread = self
                .selected_thread
                .or_else(|| snap.threads.first().map(|t| t.id));
            if let Some(thread) = thread {
                snap.frames = self.backend.frames(thread).unwrap_or_default();
                match self.backend.registers(thread) {
                    Ok(regs) => {
                        if let Some(prev) = &self.prev_registers {
                            if prev.thread_id == regs.thread_id {
                                snap.register_changes = regs.diff(prev);
                            }
                        }
                        self.prev_registers = Some(regs.clone());
                        snap.registers = Some(regs);
                    }
                    Err(_) => snap.registers = None,
                }
                let pc = snap.frames.first().map(|f| f.pc);
                snap.disassembly = self
                    .backend
                    .disassemble(pc, DISASM_WINDOW)
                    .unwrap_or_default();
            }
        } else {
            // Process not stopped: clear volatile views but keep static ones.
            self.prev_registers = None;
        }

        if snap.process.lifecycle == ProcessLifecycle::Exited {
            self.prev_registers = None;
        }

        Ok(snap)
    }
}

/// Convenience used in tests and simple flows: apply a stepping command by
/// kind. (Kept here so callers don't reconstruct the enum variants inline.)
pub fn step_command(kind: StepKind) -> Command {
    Command::Step(kind)
}

/// Register-change helper mirrored for external callers/tests.
pub fn changes_between(newer: &RegisterSet, older: &RegisterSet) -> Vec<RegisterChange> {
    newer.diff(older)
}
