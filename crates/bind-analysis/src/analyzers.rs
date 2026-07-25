//! Concrete analyses.
//!
//! Each is an incremental consumer of the event stream. They are deliberately
//! honest about confidence: stop tallies are `High` (directly counted), while
//! function transitions and stack depth are `Medium`/`Low` because they are
//! *inferred* from stepping granularity and symbol resolution.

use std::collections::HashMap;

use bind_core::{Address, DebugEvent, SequencedEvent, StopReason};

use crate::analyzer::{AnalysisContext, Analyzer};
use crate::finding::{Confidence, Finding};

/// Tallies why and how often the target stopped.
#[derive(Default)]
pub struct StopSummary {
    breakpoints: u64,
    steps: u64,
    signals: HashMap<String, u64>,
    exceptions: u64,
    exit_code: Option<i32>,
    total_stops: u64,
}

impl Analyzer for StopSummary {
    fn name(&self) -> &'static str {
        "stop-summary"
    }

    fn on_event(&mut self, event: &SequencedEvent, _ctx: &mut AnalysisContext) {
        match &event.event {
            DebugEvent::Stopped { reason, .. } => {
                self.total_stops += 1;
                match reason {
                    StopReason::Breakpoint(_) => self.breakpoints += 1,
                    StopReason::Step => self.steps += 1,
                    StopReason::Signal(s) => *self.signals.entry(s.clone()).or_default() += 1,
                    StopReason::Exception(_) => self.exceptions += 1,
                    _ => {}
                }
            }
            DebugEvent::SignalReceived { signal, .. } => {
                *self.signals.entry(signal.clone()).or_default() += 1;
            }
            DebugEvent::ProcessExited { code } => self.exit_code = Some(*code),
            _ => {}
        }
    }

    fn findings(&self) -> Vec<Finding> {
        let mut summary = format!(
            "{} stops: {} breakpoint, {} step",
            self.total_stops, self.breakpoints, self.steps
        );
        if let Some(code) = self.exit_code {
            summary.push_str(&format!("; exited {code}"));
        }
        let mut f = Finding::new(self.name(), "stop-summary", summary, Confidence::High);
        for (sig, n) in &self.signals {
            f = f.with_evidence(format!("signal {sig} x{n}"));
        }
        if self.exceptions > 0 {
            f = f.with_evidence(format!("{} exception(s)", self.exceptions));
        }
        vec![f]
    }
}

/// Counts instructions actually stepped, grouped by resolved function.
#[derive(Default)]
pub struct InstructionCounts {
    per_function: HashMap<String, u64>,
    total: u64,
}

impl Analyzer for InstructionCounts {
    fn name(&self) -> &'static str {
        "instruction-counts"
    }

    fn on_event(&mut self, event: &SequencedEvent, ctx: &mut AnalysisContext) {
        if let DebugEvent::InstructionStepped { to, .. } = &event.event {
            self.total += 1;
            let name = ctx.function_at(*to).unwrap_or_else(|| format!("<{to}>"));
            // Group by function, not offset.
            let func = name.split("+0x").next().unwrap_or(&name).to_string();
            *self.per_function.entry(func).or_default() += 1;
        }
    }

    fn findings(&self) -> Vec<Finding> {
        if self.total == 0 {
            return Vec::new();
        }
        let mut ranked: Vec<_> = self.per_function.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let hottest = ranked[0];
        let mut f = Finding::new(
            self.name(),
            "hot-function",
            format!(
                "{} stepped instruction(s); hottest: {} ({} = {:.0}%)",
                self.total,
                hottest.0,
                hottest.1,
                100.0 * *hottest.1 as f64 / self.total as f64
            ),
            Confidence::Medium,
        )
        .with_location(hottest.0.clone())
        .with_limitations("Counts only single-stepped instructions, not continued execution");
        for (func, n) in ranked.iter().take(5) {
            f = f.with_evidence(format!("{func}: {n}"));
        }
        vec![f]
    }
}

/// Records function transitions (a change of resolved function across steps)
/// and infers approximate stack depth from call/return patterns.
#[derive(Default)]
pub struct FunctionTransitions {
    last_function: Option<String>,
    transitions: Vec<(u64, String, String)>,
    call_stack: Vec<String>,
    max_depth: usize,
}

impl FunctionTransitions {
    fn base_name(name: &str) -> String {
        name.split("+0x").next().unwrap_or(name).to_string()
    }
}

impl Analyzer for FunctionTransitions {
    fn name(&self) -> &'static str {
        "function-transitions"
    }

    fn on_event(&mut self, event: &SequencedEvent, ctx: &mut AnalysisContext) {
        let addr: Option<Address> = match &event.event {
            DebugEvent::InstructionStepped { to, .. } => Some(*to),
            DebugEvent::Stopped { pc, .. } => *pc,
            _ => None,
        };
        let Some(addr) = addr else { return };
        let Some(func) = ctx.function_at(addr).map(|f| Self::base_name(&f)) else {
            return;
        };

        match &self.last_function {
            Some(prev) if *prev != func => {
                self.transitions
                    .push((event.seq.raw(), prev.clone(), func.clone()));
                // Heuristic call/return: if we return to a function already on
                // the stack, treat as a return (pop); otherwise a call (push).
                if self.call_stack.iter().rev().any(|f| f == &func) {
                    while let Some(top) = self.call_stack.last() {
                        if top == &func {
                            break;
                        }
                        self.call_stack.pop();
                    }
                } else {
                    self.call_stack.push(func.clone());
                    self.max_depth = self.max_depth.max(self.call_stack.len());
                }
                self.last_function = Some(func);
            }
            None => {
                self.call_stack.push(func.clone());
                self.max_depth = self.max_depth.max(1);
                self.last_function = Some(func);
            }
            _ => {}
        }
    }

    fn findings(&self) -> Vec<Finding> {
        if self.transitions.is_empty() {
            return Vec::new();
        }
        let mut f = Finding::new(
            self.name(),
            "function-transitions",
            format!(
                "{} function transition(s); inferred max stack depth {}",
                self.transitions.len(),
                self.max_depth
            ),
            Confidence::Low,
        )
        .with_limitations(
            "Transitions and depth are inferred from stepping + symbol changes; \
             tail calls and missing symbols reduce accuracy",
        );
        for (seq, from, to) in self.transitions.iter().take(8) {
            f.related_events.push(*seq);
            f = f.with_evidence(format!("{from} -> {to}"));
        }
        vec![f]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::AnalyzerSet;
    use bind_core::{EventSeq, ThreadId};
    use bind_symbols::SymbolIndex;

    fn stepped(seq: u64, to: u64) -> SequencedEvent {
        SequencedEvent::new(
            EventSeq::new(seq),
            seq,
            DebugEvent::InstructionStepped {
                thread: ThreadId::new(1),
                from: None,
                to: Address::new(to),
            },
        )
    }

    fn symbols() -> SymbolIndex {
        let mut s = SymbolIndex::new();
        s.insert("main", Address::new(0x1000), 0x20, None);
        s.insert("helper", Address::new(0x1020), 0x10, None);
        s
    }

    #[test]
    fn instruction_counts_group_by_function() {
        let mut syms = symbols();
        let mut set = AnalyzerSet::new()
            .with(Box::new(InstructionCounts::default()))
            .with(Box::new(FunctionTransitions::default()));
        let mut ctx = AnalysisContext::new(Some(&mut syms));
        // main, main, helper, main
        for (seq, a) in [(1, 0x1000), (2, 0x1004), (3, 0x1020), (4, 0x1008)] {
            set.on_event(&stepped(seq, a), &mut ctx);
        }
        let findings = set.findings();
        let counts = findings
            .iter()
            .find(|f| f.analyzer == "instruction-counts")
            .unwrap();
        assert!(counts.summary.contains("main"));
        let trans = findings
            .iter()
            .find(|f| f.analyzer == "function-transitions")
            .unwrap();
        // main->helper->main = 2 transitions.
        assert!(trans.summary.contains('2'));
    }

    #[test]
    fn stop_summary_counts_reasons() {
        let mut a = StopSummary::default();
        let mut ctx = AnalysisContext::empty();
        let bp = SequencedEvent::new(
            EventSeq::new(1),
            0,
            DebugEvent::Stopped {
                thread: ThreadId::new(1),
                reason: StopReason::Breakpoint(bind_core::BreakpointId::new(1)),
                pc: None,
            },
        );
        a.on_event(&bp, &mut ctx);
        let f = a.findings();
        assert!(f[0].summary.contains("1 breakpoint"));
        assert_eq!(f[0].confidence, Confidence::High);
    }
}
