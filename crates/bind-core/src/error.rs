//! Structured error hierarchy.
//!
//! The variants distinguish *categories* of failure so the UI can present the
//! right thing (a user-input mistake is not an internal invariant violation is
//! not a backend limitation). `Display` is intentionally concise and
//! human-readable; deeper diagnostic detail belongs in logs, not in the
//! user-facing string.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BindError {
    /// The user asked for something malformed (bad address, unknown symbol
    /// syntax, ...). Recoverable; show and continue.
    #[error("invalid input: {0}")]
    UserInput(String),

    /// The operation is meaningful but this backend does not implement it.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// The backend implements it, but the current target/capabilities can't.
    #[error("not available: {0}")]
    CapabilityLimited(String),

    /// The target is in the wrong state for this operation (e.g. stepping a
    /// process that is running or exited).
    #[error("invalid target state: {0}")]
    TargetState(String),

    /// The underlying debugger engine returned an error.
    #[error("backend error: {0}")]
    Backend(String),

    #[error("symbol resolution failed: {0}")]
    Symbol(String),

    #[error("memory read failed: {0}")]
    Memory(String),

    #[error("trace storage error: {0}")]
    Trace(String),

    /// A broken assumption inside Bind itself — a bug.
    #[error("internal error: {0}")]
    Internal(String),
}

impl BindError {
    /// True for errors the user can act on directly (retry with different
    /// input); false for backend/internal faults.
    pub fn is_user_actionable(&self) -> bool {
        matches!(self, BindError::UserInput(_))
    }

    /// A short category label for UI badges/logging.
    pub fn category(&self) -> &'static str {
        match self {
            BindError::UserInput(_) => "input",
            BindError::Unsupported(_) => "unsupported",
            BindError::CapabilityLimited(_) => "unavailable",
            BindError::TargetState(_) => "state",
            BindError::Backend(_) => "backend",
            BindError::Symbol(_) => "symbol",
            BindError::Memory(_) => "memory",
            BindError::Trace(_) => "trace",
            BindError::Internal(_) => "internal",
        }
    }
}

pub type BindResult<T> = Result<T, BindError>;
