//! Immutable session snapshots.
//!
//! The application thread maintains one of these by folding the event stream
//! and the results of data-fetch commands. The TUI renders *from* a borrowed
//! snapshot and never talks to the debugger directly, which keeps rendering off
//! the debugger lock and makes views trivially testable.

use serde::{Deserialize, Serialize};

use crate::capability::Capabilities;
use crate::id::SessionId;
use crate::model::{
    Breakpoint, Frame, Instruction, MemoryRegion, Module, ProcessInfo, RegisterChange, RegisterSet,
    ThreadInfo, Watchpoint,
};

/// A consistent, cheap-to-clone read model of the current debugging session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session: SessionId,
    pub backend_name: String,
    pub capabilities: Capabilities,
    pub process: ProcessInfo,
    pub program: Option<String>,
    pub threads: Vec<ThreadInfo>,
    pub frames: Vec<Frame>,
    pub registers: Option<RegisterSet>,
    /// Register changes relative to the previous stop, for highlighting.
    pub register_changes: Vec<RegisterChange>,
    pub disassembly: Vec<Instruction>,
    pub breakpoints: Vec<Breakpoint>,
    pub watchpoints: Vec<Watchpoint>,
    pub modules: Vec<Module>,
    pub memory_regions: Vec<MemoryRegion>,
    /// Total number of events observed this session (the timeline may retain
    /// only a bounded suffix of these).
    pub event_count: u64,
    /// Events dropped/sampled out of retention, surfaced honestly.
    pub events_dropped: u64,
}

impl SessionSnapshot {
    pub fn empty(session: SessionId, backend_name: impl Into<String>, caps: Capabilities) -> Self {
        Self {
            session,
            backend_name: backend_name.into(),
            capabilities: caps,
            process: ProcessInfo::default(),
            program: None,
            threads: Vec::new(),
            frames: Vec::new(),
            registers: None,
            register_changes: Vec::new(),
            disassembly: Vec::new(),
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            modules: Vec::new(),
            memory_regions: Vec::new(),
            event_count: 0,
            events_dropped: 0,
        }
    }

    pub fn selected_frame(&self) -> Option<&Frame> {
        self.frames
            .iter()
            .find(|f| f.is_selected)
            .or_else(|| self.frames.first())
    }

    pub fn selected_thread(&self) -> Option<&ThreadInfo> {
        self.threads
            .iter()
            .find(|t| t.is_selected)
            .or_else(|| self.threads.first())
    }
}
