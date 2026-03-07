use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type SessionId = u64;
pub type ThreadId = u64;
pub type FrameId = u64;
pub type Address = u64;
pub type BreakpointId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuArch {
    X86_64,
    AArch64,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endianness {
    Little,
    Big,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetBinary {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessState {
    pub pid: Option<u32>,
    pub state: ProcessLifecycle,
    pub stop_reason: StopReason,
    pub arch: CpuArch,
    pub pointer_width_bits: u8,
    pub endianness: Endianness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessLifecycle {
    Idle,
    Launching,
    Running,
    Stopped,
    Exited,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugSession {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub target: Option<TargetBinary>,
    pub process: ProcessState,
    pub threads: Vec<ThreadState>,
    pub frames: Vec<FrameState>,
    pub breakpoints: Vec<Breakpoint>,
    pub watchpoints: Vec<Watchpoint>,
    pub disassembly: Vec<DisassemblyLine>,
    pub trace_events: Vec<TraceEvent>,
    pub output_log: Vec<String>,
}

impl DebugSession {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            created_at: Utc::now(),
            target: None,
            process: ProcessState {
                pid: None,
                state: ProcessLifecycle::Idle,
                stop_reason: StopReason::None,
                arch: CpuArch::Unknown,
                pointer_width_bits: 64,
                endianness: Endianness::Little,
            },
            threads: Vec::new(),
            frames: Vec::new(),
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            disassembly: Vec::new(),
            trace_events: Vec::new(),
            output_log: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadState {
    pub id: ThreadId,
    pub name: Option<String>,
    pub is_current: bool,
    pub status: ThreadStatus,
    pub stop_reason: StopReason,
    pub instruction_pointer: Option<Address>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadStatus {
    Running,
    Stopped,
    Sleeping,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameState {
    pub id: FrameId,
    pub thread_id: ThreadId,
    pub index: usize,
    pub instruction_pointer: Address,
    pub stack_pointer: Option<Address>,
    pub frame_pointer: Option<Address>,
    pub function: Option<String>,
    pub symbol: Option<SymbolInfo>,
    pub source: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterBank {
    pub thread_id: ThreadId,
    pub bank_name: String,
    pub registers: Vec<RegisterValue>,
}

impl RegisterBank {
    pub fn diff(&self, older: &RegisterBank) -> Vec<RegisterDiff> {
        let mut diffs = Vec::new();

        for reg in &self.registers {
            if let Some(prev) = older
                .registers
                .iter()
                .find(|candidate| candidate.name == reg.name)
            {
                if prev.value != reg.value {
                    diffs.push(RegisterDiff {
                        name: reg.name.clone(),
                        old_value: prev.value.clone(),
                        new_value: reg.value.clone(),
                    });
                }
            }
        }

        diffs
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterDiff {
    pub name: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterValue {
    pub name: String,
    pub value: String,
    pub size_bits: u16,
    pub role: RegisterRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterRole {
    General,
    InstructionPointer,
    StackPointer,
    FramePointer,
    Flags,
    Vector,
    Floating,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRegion {
    pub start: Address,
    pub end: Address,
    pub permissions: MemoryPermissions,
    pub pathname: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPermissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub base: Address,
    pub bytes: Vec<u8>,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    pub id: BreakpointId,
    pub location: BreakpointLocation,
    pub enabled: bool,
    pub hit_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointLocation {
    Address(Address),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watchpoint {
    pub id: u64,
    pub address: Address,
    pub size: u64,
    pub access: WatchAccess,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionRecord {
    pub address: Address,
    pub bytes: Vec<u8>,
    pub mnemonic: String,
    pub operands: String,
    pub symbol: Option<SymbolInfo>,
    pub source: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    None,
    Breakpoint(BreakpointId),
    Watchpoint(u64),
    Step,
    Signal(String),
    Exception(String),
    Exited(i32),
    Detached,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisassemblyLine {
    pub address: Address,
    pub bytes: Vec<u8>,
    pub mnemonic: String,
    pub operands: String,
    pub function: Option<String>,
    pub source: Option<SourceLocation>,
    pub branch_target: Option<Address>,
    pub is_current: bool,
    pub has_breakpoint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub name: String,
    pub module: Option<String>,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub timestamp: DateTime<Utc>,
    pub thread_id: ThreadId,
    pub instruction: InstructionRecord,
    pub stop_reason: StopReason,
    pub delta: Option<ExecutionDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDelta {
    pub changed_registers: Vec<RegisterDiff>,
    pub control_flow: ControlFlowChange,
    pub stack_pointer_delta: i64,
    pub memory_writes: Vec<MemoryWrite>,
    pub hints: Vec<ExecutionHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlFlowChange {
    Linear,
    Jump,
    Call,
    Return,
    Syscall,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryWrite {
    pub address: Address,
    pub old_bytes: Vec<u8>,
    pub new_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionHint {
    Call,
    Return,
    Jump,
    ConditionalBranch,
    Prologue,
    Epilogue,
    Syscall,
}
