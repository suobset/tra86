//! Typed commands sent from the UI/command layer down to the debugger worker.
//!
//! Nothing in this enum is a raw debugger console string. The command palette
//! and keybindings both produce these; the LLDB backend translates them into
//! programmatic API calls. A separate, clearly-labelled raw-console escape
//! hatch may exist elsewhere, but core workflows never go through string
//! concatenation into a debugger.

use serde::{Deserialize, Serialize};

use crate::address::Address;
use crate::id::{BreakpointId, FrameId, ThreadId};
use crate::model::{AttachSpec, BreakpointLocation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    /// Source-level step into calls.
    Into,
    /// Source-level step over calls.
    Over,
    /// Run until the current function returns.
    Out,
    /// Single machine instruction.
    Instruction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Load and start controlling a target (launch / attach / core).
    Attach(AttachSpec),
    Continue,
    Pause,
    Step(StepKind),
    /// Run until an address or source line is reached.
    RunTo(RunTarget),
    Detach,
    Terminate,

    SelectThread(ThreadId),
    SelectFrame(FrameId),

    AddBreakpoint {
        location: BreakpointLocation,
        condition: Option<String>,
    },
    RemoveBreakpoint(BreakpointId),
    EnableBreakpoint(BreakpointId, bool),

    /// Read `len` bytes of memory starting at `addr`.
    ReadMemory {
        addr: Address,
        len: usize,
    },
    /// Refresh the register set of the selected/given thread.
    ReadRegisters(Option<ThreadId>),
    /// Disassemble `count` instructions around an address (or current pc).
    Disassemble {
        around: Option<Address>,
        count: usize,
    },

    /// Begin recording a trace to the given path (None = in-memory only).
    TraceStart {
        path: Option<String>,
    },
    TraceStop,

    /// Graceful shutdown of the worker.
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunTarget {
    Address(Address),
    Source { file: String, line: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrips() {
        let c = Command::Step(StepKind::Instruction);
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&j).unwrap(), c);
    }
}
