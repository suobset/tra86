//! The incremental analysis interface.
//!
//! Analyzers consume the normalized event stream one event at a time and
//! accumulate state, rather than rescanning a full trace. This keeps analysis
//! cheap enough to run live and identical between live and replay sessions.

use bind_core::{Address, SequencedEvent};
use bind_symbols::SymbolIndex;

use crate::finding::Finding;

/// Read/resolve context handed to analyzers. Symbol resolution is optional so
/// analyses still run (with reduced insight) on stripped targets.
pub struct AnalysisContext<'a> {
    symbols: Option<&'a mut SymbolIndex>,
}

impl<'a> AnalysisContext<'a> {
    pub fn new(symbols: Option<&'a mut SymbolIndex>) -> Self {
        Self { symbols }
    }

    pub fn empty() -> Self {
        Self { symbols: None }
    }

    /// Resolve a function name for an address, if symbols are available.
    pub fn function_at(&mut self, addr: Address) -> Option<String> {
        self.symbols.as_mut().and_then(|s| s.function_at(addr))
    }
}

pub trait Analyzer {
    fn name(&self) -> &'static str;
    fn on_event(&mut self, event: &SequencedEvent, ctx: &mut AnalysisContext);
    fn findings(&self) -> Vec<Finding>;
}

/// Runs a set of analyzers over one event stream.
#[derive(Default)]
pub struct AnalyzerSet {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalyzerSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, a: Box<dyn Analyzer>) -> Self {
        self.analyzers.push(a);
        self
    }

    pub fn push(&mut self, a: Box<dyn Analyzer>) {
        self.analyzers.push(a);
    }

    pub fn on_event(&mut self, event: &SequencedEvent, ctx: &mut AnalysisContext) {
        for a in &mut self.analyzers {
            a.on_event(event, ctx);
        }
    }

    /// All findings across analyzers.
    pub fn findings(&self) -> Vec<Finding> {
        self.analyzers.iter().flat_map(|a| a.findings()).collect()
    }

    pub fn len(&self) -> usize {
        self.analyzers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.analyzers.is_empty()
    }
}
