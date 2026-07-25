//! `bind-analysis`: derived runtime insight over the event stream.
//!
//! Analyses are incremental [`Analyzer`]s that fold the normalized event stream
//! and emit [`Finding`]s carrying explicit [`Confidence`] and limitations. A
//! few are shipped correctly rather than many half-done: [`StopSummary`],
//! [`InstructionCounts`], [`FunctionTransitions`], plus the snapshot-fed
//! [`RegisterChurn`]. The [`classify`] module provides arch-aware control-flow
//! hints (generalized from tra86's x86-only heuristics).

pub mod analyzer;
pub mod analyzers;
pub mod classify;
pub mod finding;
pub mod register;

pub use analyzer::{AnalysisContext, Analyzer, AnalyzerSet};
pub use analyzers::{FunctionTransitions, InstructionCounts, StopSummary};
pub use classify::{classify, ControlFlow};
pub use finding::{Confidence, Finding};
pub use register::RegisterChurn;

/// Constructs the default analyzer set Bind runs in a live session.
pub fn default_analyzers() -> AnalyzerSet {
    AnalyzerSet::new()
        .with(Box::new(StopSummary::default()))
        .with(Box::new(InstructionCounts::default()))
        .with(Box::new(FunctionTransitions::default()))
}
