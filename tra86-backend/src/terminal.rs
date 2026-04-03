use std::sync::Arc;

use crate::BackendError;

pub trait TerminalIo: Send + Sync {
    fn write_input(&self, input: &str) -> Result<(), BackendError>;
    fn drain_output(&self) -> String;
}

pub type SharedTerminal = Arc<dyn TerminalIo>;
