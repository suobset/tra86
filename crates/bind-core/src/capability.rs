//! Backend capability reporting.
//!
//! tra86 used a `BTreeMap<String, bool>` here, which was untyped "architecture
//! theater". Bind uses an explicit struct: every capability is a named field
//! with documented meaning, so the UI can honestly grey out actions a backend
//! or target cannot perform, and so JIT/watchpoint support is *reported* rather
//! than assumed.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub launch: bool,
    pub attach: bool,
    pub core_files: bool,
    pub detach: bool,
    /// Reliable asynchronous pause of a running process.
    pub async_pause: bool,
    pub step_instruction: bool,
    pub step_source: bool,
    pub breakpoints: bool,
    pub conditional_breakpoints: bool,
    pub watchpoints: bool,
    pub read_memory: bool,
    pub write_memory: bool,
    pub disassemble: bool,
    pub registers: bool,
    /// Backend surfaces JIT / dynamically-generated code load notifications.
    pub jit_events: bool,
    pub symbol_resolution: bool,
    pub source_resolution: bool,
}

impl Capabilities {
    /// A conservative "nothing supported" baseline for backends to override.
    pub const NONE: Capabilities = Capabilities {
        launch: false,
        attach: false,
        core_files: false,
        detach: false,
        async_pause: false,
        step_instruction: false,
        step_source: false,
        breakpoints: false,
        conditional_breakpoints: false,
        watchpoints: false,
        read_memory: false,
        write_memory: false,
        disassemble: false,
        registers: false,
        jit_events: false,
        symbol_resolution: false,
        source_resolution: false,
    };
}
