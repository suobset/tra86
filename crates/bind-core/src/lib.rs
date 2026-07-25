//! `bind-core`: the debugger-independent domain model for Bind.
//!
//! Everything here describes debugging *concepts* and carries no dependency on
//! LLDB, any TUI, subprocess management, or storage engines. The dependency
//! arrows in the workspace all point inward to this crate.
//!
//! Module map:
//! - [`id`]     — type-distinct newtype identifiers
//! - [`address`]— [`Address`] / [`AddressRange`]
//! - [`arch`]   — architecture & language metadata (the only place with
//!   hard-coded register-name knowledge)
//! - [`model`]  — threads, frames, registers, breakpoints, instructions, ...
//! - [`event`]  — the normalized [`DebugEvent`] stream
//! - [`command`]— typed [`Command`]s driven by UI/keybindings
//! - [`capability`] — explicit backend [`Capabilities`]
//! - [`error`]  — the [`BindError`] hierarchy
//! - [`snapshot`] — the immutable [`SessionSnapshot`] the TUI renders

pub mod address;
pub mod arch;
pub mod capability;
pub mod command;
pub mod error;
pub mod event;
pub mod id;
pub mod model;
pub mod snapshot;

pub use address::{Address, AddressRange};
pub use arch::{Architecture, Endianness, Language};
pub use capability::Capabilities;
pub use command::{Command, RunTarget, StepKind};
pub use error::{BindError, BindResult};
pub use event::{DebugEvent, SequencedEvent};
pub use id::{BreakpointId, EventSeq, FrameId, ModuleId, Pid, SessionId, ThreadId, WatchpointId};
pub use model::{
    AttachSpec, Breakpoint, BreakpointLocation, Frame, Instruction, LanguageInfo,
    MemoryPermissions, MemoryRegion, Module, ProcessInfo, ProcessLifecycle, Register,
    RegisterChange, RegisterRole, RegisterSet, ResolvedLocation, SourceLocation, StopReason,
    Symbol, TargetSpec, ThreadInfo, ThreadRunState, WatchKind, Watchpoint,
};
pub use snapshot::SessionSnapshot;
