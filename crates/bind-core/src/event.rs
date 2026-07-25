//! The normalized debugger event stream.
//!
//! Every backend notification and every completed command produces one or more
//! [`DebugEvent`]s. The TUI, the trace recorder, and the analysis engine all
//! consume this single stream rather than independently polling and
//! interpreting backend state. This is the core correction over tra86, where
//! each consumer re-queried and re-parsed debugger output on its own.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::Address;
use crate::arch::Architecture;
use crate::id::{BreakpointId, EventSeq, ModuleId, Pid, ThreadId, WatchpointId};
use crate::model::{Module, StopReason};

/// The kind and payload of a normalized debugger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebugEvent {
    /// A target was created/opened but not yet running.
    TargetLoaded {
        program: String,
        arch: Architecture,
    },
    ProcessLaunched {
        pid: Pid,
    },
    ProcessAttached {
        pid: Pid,
    },
    ProcessExited {
        code: i32,
    },
    ProcessDetached,
    /// The process resumed running.
    Continued,
    /// The process stopped. Snapshots (registers, frames, ...) are fetched
    /// lazily by consumers, not embedded here, to keep events small.
    Stopped {
        thread: ThreadId,
        reason: StopReason,
        pc: Option<Address>,
    },
    ThreadCreated {
        thread: ThreadId,
    },
    ThreadExited {
        thread: ThreadId,
    },
    BreakpointHit {
        breakpoint: BreakpointId,
        thread: ThreadId,
    },
    BreakpointResolved {
        breakpoint: BreakpointId,
        address: Address,
    },
    WatchpointHit {
        watchpoint: WatchpointId,
        thread: ThreadId,
    },
    SignalReceived {
        thread: ThreadId,
        signal: String,
    },
    ModuleLoaded {
        module: Module,
    },
    ModuleUnloaded {
        module: ModuleId,
    },
    /// Runtime-generated code became available (JIT). Kept distinct from
    /// `ModuleLoaded` so analyses can treat it honestly.
    JitCodeLoaded {
        module: Module,
    },
    /// Emitted after a single-instruction step completes, carrying the address
    /// that was executed to reach the new stop.
    InstructionStepped {
        thread: ThreadId,
        from: Option<Address>,
        to: Address,
    },
    /// A backend/worker level diagnostic that is not fatal to the session.
    Notice {
        message: String,
    },
    /// A recoverable error surfaced as an event rather than swallowed.
    Error {
        message: String,
    },
}

impl DebugEvent {
    /// The thread this event pertains to, if any.
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            DebugEvent::Stopped { thread, .. }
            | DebugEvent::ThreadCreated { thread }
            | DebugEvent::ThreadExited { thread }
            | DebugEvent::BreakpointHit { thread, .. }
            | DebugEvent::WatchpointHit { thread, .. }
            | DebugEvent::SignalReceived { thread, .. }
            | DebugEvent::InstructionStepped { thread, .. } => Some(*thread),
            _ => None,
        }
    }

    /// A short, stable kind label for grouping/filtering/UI.
    pub fn kind(&self) -> &'static str {
        match self {
            DebugEvent::TargetLoaded { .. } => "target-loaded",
            DebugEvent::ProcessLaunched { .. } => "process-launched",
            DebugEvent::ProcessAttached { .. } => "process-attached",
            DebugEvent::ProcessExited { .. } => "process-exited",
            DebugEvent::ProcessDetached => "process-detached",
            DebugEvent::Continued => "continued",
            DebugEvent::Stopped { .. } => "stopped",
            DebugEvent::ThreadCreated { .. } => "thread-created",
            DebugEvent::ThreadExited { .. } => "thread-exited",
            DebugEvent::BreakpointHit { .. } => "breakpoint-hit",
            DebugEvent::BreakpointResolved { .. } => "breakpoint-resolved",
            DebugEvent::WatchpointHit { .. } => "watchpoint-hit",
            DebugEvent::SignalReceived { .. } => "signal",
            DebugEvent::ModuleLoaded { .. } => "module-loaded",
            DebugEvent::ModuleUnloaded { .. } => "module-unloaded",
            DebugEvent::JitCodeLoaded { .. } => "jit-code-loaded",
            DebugEvent::InstructionStepped { .. } => "instruction-stepped",
            DebugEvent::Notice { .. } => "notice",
            DebugEvent::Error { .. } => "error",
        }
    }
}

/// A `DebugEvent` placed in the ordered stream: it carries a monotonic sequence
/// number and timestamps. This is what gets stored in the trace and shown in
/// the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub seq: EventSeq,
    /// Monotonic timestamp in nanoseconds since session start.
    pub mono_nanos: u64,
    /// Optional wall-clock time (may be dropped from persisted traces).
    pub wall: Option<DateTime<Utc>>,
    pub event: DebugEvent,
}

impl SequencedEvent {
    pub fn new(seq: EventSeq, mono_nanos: u64, event: DebugEvent) -> Self {
        Self {
            seq,
            mono_nanos,
            wall: Some(Utc::now()),
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_thread_extraction() {
        let e = DebugEvent::Stopped {
            thread: ThreadId::new(3),
            reason: StopReason::Step,
            pc: Some(Address::new(0x1000)),
        };
        assert_eq!(e.thread(), Some(ThreadId::new(3)));
        assert_eq!(DebugEvent::Continued.thread(), None);
    }

    #[test]
    fn event_roundtrips_through_json() {
        let e = DebugEvent::BreakpointHit {
            breakpoint: BreakpointId::new(2),
            thread: ThreadId::new(1),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: DebugEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
