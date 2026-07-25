//! The backend contract.
//!
//! A [`DebugBackend`] maps some concrete debugger engine (LLDB first, but the
//! trait names no LLDB type) onto `bind-core`'s domain model. It is a
//! *synchronous* trait: the [`crate::worker`] owns the backend on a dedicated
//! thread and pumps it, so blocking calls here never touch the UI thread.
//!
//! Backends expose asynchronously-arriving notifications (process stopped,
//! thread created, module loaded, ...) by buffering them and returning them
//! from [`DebugBackend::poll_events`], which the worker drains after every
//! command and on idle ticks.

use bind_core::BindResult;
use bind_core::{
    Address, AttachSpec, Breakpoint, BreakpointId, BreakpointLocation, Capabilities, DebugEvent,
    Frame, Instruction, MemoryRegion, Module, ProcessInfo, RegisterSet, RunTarget, StepKind,
    ThreadId, ThreadInfo,
};

pub trait DebugBackend: Send {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    // --- target / process lifecycle ---
    fn attach(&mut self, spec: &AttachSpec) -> BindResult<()>;
    fn detach(&mut self) -> BindResult<()>;
    fn terminate(&mut self) -> BindResult<()>;

    // --- execution control ---
    fn resume(&mut self) -> BindResult<()>;
    fn pause(&mut self) -> BindResult<()>;
    fn step(&mut self, kind: StepKind) -> BindResult<()>;
    fn run_to(&mut self, target: &RunTarget) -> BindResult<()>;

    // --- selection ---
    fn select_thread(&mut self, thread: ThreadId) -> BindResult<()>;
    fn select_frame(&mut self, frame_index: usize) -> BindResult<()>;

    // --- breakpoints ---
    fn add_breakpoint(
        &mut self,
        location: &BreakpointLocation,
        condition: Option<&str>,
    ) -> BindResult<Breakpoint>;
    fn remove_breakpoint(&mut self, id: BreakpointId) -> BindResult<()>;
    fn enable_breakpoint(&mut self, id: BreakpointId, enabled: bool) -> BindResult<()>;
    fn breakpoints(&mut self) -> BindResult<Vec<Breakpoint>>;

    // --- state queries (only valid while stopped) ---
    fn process_info(&mut self) -> BindResult<ProcessInfo>;
    fn threads(&mut self) -> BindResult<Vec<ThreadInfo>>;
    fn frames(&mut self, thread: ThreadId) -> BindResult<Vec<Frame>>;
    fn registers(&mut self, thread: ThreadId) -> BindResult<RegisterSet>;
    fn read_memory(&mut self, addr: Address, len: usize) -> BindResult<Vec<u8>>;
    fn disassemble(
        &mut self,
        around: Option<Address>,
        count: usize,
    ) -> BindResult<Vec<Instruction>>;
    fn modules(&mut self) -> BindResult<Vec<Module>>;

    fn memory_regions(&mut self) -> BindResult<Vec<MemoryRegion>> {
        Ok(Vec::new())
    }

    /// Non-blocking drain of any normalized events that have accumulated since
    /// the last call. Backends must never block here.
    fn poll_events(&mut self) -> Vec<DebugEvent>;
}
