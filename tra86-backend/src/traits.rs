use std::collections::BTreeMap;

use tra86_core::{
    Address, Breakpoint, BreakpointId, BreakpointLocation, DisassemblyLine, FrameState,
    MemoryRegion, RegisterBank, SourceLocation, StopReason, SymbolInfo, TargetBinary, ThreadId,
    ThreadState,
};

use crate::BackendError;

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub target: TargetBinary,
}

/// Normalized debugger/tracer backend surface used by the UI and analysis layers.
///
/// Backends map native protocol concepts (LLDB, GDB/MI, dbgeng, or future instrumentation)
/// into `tra86-core` model types so higher layers never depend on backend-specific structs.
pub trait DebugBackend: Send {
    fn backend_name(&self) -> &'static str;

    fn open_target(&mut self, _program: &str) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(
            "open_target is not implemented by this backend".to_string(),
        ))
    }

    fn launch(&mut self, request: LaunchRequest) -> Result<(), BackendError>;
    fn attach(&mut self, pid: u32) -> Result<(), BackendError>;
    fn detach(&mut self) -> Result<(), BackendError>;
    fn kill(&mut self) -> Result<(), BackendError>;

    fn continue_exec(&mut self) -> Result<(), BackendError>;
    fn step_into(&mut self) -> Result<(), BackendError>;
    fn step_over(&mut self) -> Result<(), BackendError>;
    fn step_out(&mut self) -> Result<(), BackendError>;
    fn pause(&mut self) -> Result<(), BackendError>;

    fn read_registers(&mut self, thread_id: ThreadId) -> Result<RegisterBank, BackendError>;
    fn read_memory(&mut self, address: Address, length: usize) -> Result<Vec<u8>, BackendError>;
    fn write_memory(&mut self, address: Address, bytes: &[u8]) -> Result<(), BackendError>;

    fn list_threads(&mut self) -> Result<Vec<ThreadState>, BackendError>;
    fn list_frames(&mut self, thread_id: ThreadId) -> Result<Vec<FrameState>, BackendError>;

    fn set_breakpoint(&mut self, location: BreakpointLocation) -> Result<Breakpoint, BackendError>;
    fn remove_breakpoint(&mut self, id: BreakpointId) -> Result<(), BackendError>;

    fn disassemble(
        &mut self,
        address: Option<Address>,
        count: usize,
    ) -> Result<Vec<DisassemblyLine>, BackendError>;
    fn memory_map(&mut self) -> Result<Vec<MemoryRegion>, BackendError>;

    fn current_instruction(&mut self, thread_id: ThreadId)
        -> Result<Option<Address>, BackendError>;
    fn current_stop_reason(&mut self) -> Result<StopReason, BackendError>;

    fn source_location(&mut self, address: Address)
        -> Result<Option<SourceLocation>, BackendError>;
    fn symbolicate(&mut self, address: Address) -> Result<Option<SymbolInfo>, BackendError>;

    fn capabilities(&self) -> BTreeMap<String, bool> {
        BTreeMap::new()
    }
}
