//! `bind-debugger`: the backend contract and debugger orchestration.
//!
//! - [`DebugBackend`] is the trait every engine implements (LLDB lives in
//!   `bind-lldb`).
//! - [`MockBackend`] is a deterministic in-memory backend used for tests and
//!   TUI development.
//! - [`WorkerHandle`] / the worker thread own the backend and expose it to the
//!   application through typed [`bind_core::Command`]s and [`WorkerUpdate`]s,
//!   keeping the debugger off the render thread.

pub mod backend;
pub mod mock;
pub mod worker;

pub use backend::DebugBackend;
pub use mock::MockBackend;
pub use worker::{changes_between, step_command, WorkerHandle, WorkerUpdate};
