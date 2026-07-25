//! Debugger-independent domain model.
//!
//! These types describe debugging *concepts* (threads, frames, registers,
//! breakpoints, ...) rather than any backend's classes. Backends translate
//! their native representation into these; the TUI, trace, and analysis layers
//! only ever see these.

use serde::{Deserialize, Serialize};

use crate::address::Address;
use crate::arch::{Architecture, Endianness, Language};
use crate::id::{BreakpointId, FrameId, ModuleId, Pid, ThreadId, WatchpointId};

/// A program Bind has been asked to debug, before it is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Explicit environment overrides. Empty means "inherit".
    pub env: Vec<(String, String)>,
    pub stop_at_entry: bool,
}

impl TargetSpec {
    pub fn program(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            stop_at_entry: false,
        }
    }
}

/// How a target is attached to the debugger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachSpec {
    /// Launch a fresh process from a target spec.
    Launch(TargetSpec),
    /// Attach to a running process by pid.
    Pid(Pid),
    /// Open a core file for post-mortem inspection.
    CoreFile { program: String, core: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessLifecycle {
    /// No process yet.
    Idle,
    Launching,
    Running,
    Stopped,
    Exited,
    Detached,
    Crashed,
}

impl ProcessLifecycle {
    pub fn is_alive(self) -> bool {
        matches!(
            self,
            ProcessLifecycle::Launching
                | ProcessLifecycle::Running
                | ProcessLifecycle::Stopped
                | ProcessLifecycle::Crashed
        )
    }

    pub fn is_stopped(self) -> bool {
        matches!(self, ProcessLifecycle::Stopped | ProcessLifecycle::Crashed)
    }
}

/// High-level, backend-independent process state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: Option<Pid>,
    pub lifecycle: ProcessLifecycle,
    pub arch: Architecture,
    pub endianness: Endianness,
    pub stop_reason: StopReason,
    pub exit_code: Option<i32>,
}

impl Default for ProcessInfo {
    fn default() -> Self {
        Self {
            pid: None,
            lifecycle: ProcessLifecycle::Idle,
            arch: Architecture::Unknown,
            endianness: Endianness::Little,
            stop_reason: StopReason::None,
            exit_code: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadRunState {
    Running,
    Stopped,
    Waiting,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ThreadId,
    pub name: Option<String>,
    pub is_selected: bool,
    pub run_state: ThreadRunState,
    pub stop_reason: StopReason,
    pub pc: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub id: FrameId,
    pub thread_id: ThreadId,
    pub index: usize,
    pub pc: Address,
    pub sp: Option<Address>,
    pub fp: Option<Address>,
    pub function: Option<String>,
    pub symbol: Option<Symbol>,
    pub source: Option<SourceLocation>,
    /// True when this frame is an inlined call site rather than a physical
    /// stack frame.
    pub is_inlined: bool,
    pub is_selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterRole {
    General,
    ProgramCounter,
    StackPointer,
    FramePointer,
    Flags,
    Vector,
    Float,
    Other,
}

/// A single register value. `value` is kept as an unsigned integer when the
/// register fits in 64 bits (the common case); wider registers carry their
/// bytes in `wide` and set `value` to the low 64 bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register {
    pub name: String,
    pub value: u64,
    pub size_bits: u16,
    pub role: RegisterRole,
    /// Full little-endian bytes for registers wider than 64 bits, if provided.
    pub wide: Option<Vec<u8>>,
}

impl Register {
    pub fn new(name: impl Into<String>, value: u64, size_bits: u16, role: RegisterRole) -> Self {
        Self {
            name: name.into(),
            value,
            size_bits,
            role,
            wide: None,
        }
    }

    /// Hex rendering respecting the register width (min 64 bits shown as 16
    /// hex digits).
    pub fn hex(&self) -> String {
        let digits = (self.size_bits.max(1) as usize).div_ceil(4);
        format!("0x{:0width$x}", self.value, width = digits.max(1))
    }
}

/// The registers of one thread at one stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSet {
    pub thread_id: ThreadId,
    pub registers: Vec<Register>,
}

impl RegisterSet {
    pub fn get(&self, name: &str) -> Option<&Register> {
        self.registers.iter().find(|r| r.name == name)
    }

    pub fn by_role(&self, role: RegisterRole) -> Option<&Register> {
        self.registers.iter().find(|r| r.role == role)
    }

    /// Registers whose value differs from `older`, matched by name.
    pub fn diff(&self, older: &RegisterSet) -> Vec<RegisterChange> {
        let mut changes = Vec::new();
        for reg in &self.registers {
            if let Some(prev) = older.get(&reg.name) {
                if prev.value != reg.value || prev.wide != reg.wide {
                    changes.push(RegisterChange {
                        name: reg.name.clone(),
                        old_value: prev.value,
                        new_value: reg.value,
                    });
                }
            }
        }
        changes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterChange {
    pub name: String,
    pub old_value: u64,
    pub new_value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPermissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRegion {
    pub start: Address,
    pub end: Address,
    pub permissions: MemoryPermissions,
    pub name: Option<String>,
}

/// A loaded module (main executable or shared library).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub id: ModuleId,
    pub path: String,
    pub load_address: Option<Address>,
    /// True when the module was created at runtime (JIT / dynamically emitted
    /// object) rather than loaded from a persistent file on disk.
    pub is_jit: bool,
    pub has_debug_info: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    /// Human-readable demangled name when available.
    pub demangled: Option<String>,
    pub module: Option<String>,
    pub start: Option<Address>,
    /// Offset of the referring address from the symbol start.
    pub offset: u64,
}

impl Symbol {
    /// Preferred display form: demangled name if present, else raw.
    pub fn display(&self) -> &str {
        self.demangled.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
}

/// A single disassembled instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub address: Address,
    pub bytes: Vec<u8>,
    pub mnemonic: String,
    pub operands: String,
    pub symbol: Option<Symbol>,
    pub source: Option<SourceLocation>,
    /// Resolved branch/call target where the backend can compute it.
    pub branch_target: Option<Address>,
    pub is_current: bool,
    pub has_breakpoint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointLocation {
    Address(Address),
    Symbol(String),
    Source { file: String, line: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLocation {
    pub address: Address,
    pub symbol: Option<Symbol>,
    pub source: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    pub id: BreakpointId,
    pub location: BreakpointLocation,
    pub enabled: bool,
    pub hit_count: u64,
    pub condition: Option<String>,
    /// Concrete addresses this logical breakpoint resolved to (may be several,
    /// e.g. an inlined or templated function).
    pub resolved: Vec<ResolvedLocation>,
}

impl Breakpoint {
    pub fn is_resolved(&self) -> bool {
        !self.resolved.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchKind {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watchpoint {
    pub id: WatchpointId,
    pub address: Address,
    pub size: u64,
    pub kind: WatchKind,
    pub enabled: bool,
    pub hit_count: u64,
}

/// Why the target stopped. Backend-native reasons are normalized into this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    None,
    Breakpoint(BreakpointId),
    Watchpoint(WatchpointId),
    Step,
    Signal(String),
    Exception(String),
    Exited(i32),
    Detached,
    Unknown(String),
}

impl StopReason {
    pub fn short(&self) -> String {
        match self {
            StopReason::None => "none".into(),
            StopReason::Breakpoint(id) => format!("breakpoint #{id}"),
            StopReason::Watchpoint(id) => format!("watchpoint #{id}"),
            StopReason::Step => "step".into(),
            StopReason::Signal(s) => format!("signal {s}"),
            StopReason::Exception(e) => format!("exception {e}"),
            StopReason::Exited(code) => format!("exited {code}"),
            StopReason::Detached => "detached".into(),
            StopReason::Unknown(s) => format!("stop ({s})"),
        }
    }
}

/// Metadata Bind knows about the target/source language of the current frame or
/// process. Optional throughout — never required for correct operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LanguageInfo {
    pub language: Language,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_hex_respects_width() {
        let r = Register::new("w0", 0xab, 32, RegisterRole::General);
        assert_eq!(r.hex(), "0x000000ab");
        let r64 = Register::new("x0", 0xab, 64, RegisterRole::General);
        assert_eq!(r64.hex(), "0x00000000000000ab");
    }

    #[test]
    fn register_diff_matches_by_name() {
        let older = RegisterSet {
            thread_id: ThreadId::new(1),
            registers: vec![
                Register::new("x0", 1, 64, RegisterRole::General),
                Register::new("x1", 2, 64, RegisterRole::General),
            ],
        };
        let newer = RegisterSet {
            thread_id: ThreadId::new(1),
            registers: vec![
                Register::new("x0", 9, 64, RegisterRole::General),
                Register::new("x1", 2, 64, RegisterRole::General),
            ],
        };
        let changes = newer.diff(&older);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "x0");
        assert_eq!(changes[0].old_value, 1);
        assert_eq!(changes[0].new_value, 9);
    }

    #[test]
    fn lifecycle_predicates() {
        assert!(ProcessLifecycle::Stopped.is_stopped());
        assert!(ProcessLifecycle::Stopped.is_alive());
        assert!(!ProcessLifecycle::Exited.is_alive());
    }
}
