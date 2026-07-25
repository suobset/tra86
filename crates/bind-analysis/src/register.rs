//! Register-change summarization.
//!
//! Register values live in [`bind_core::RegisterSet`] snapshots rather than in
//! the event stream, so this analysis is a small stateful helper the
//! application feeds with successive register sets (typically one per stop).

use std::collections::HashMap;

use bind_core::RegisterSet;

/// Tracks how often each register changed across stops.
#[derive(Default)]
pub struct RegisterChurn {
    prev: Option<RegisterSet>,
    changes: HashMap<String, u64>,
    stops: u64,
}

impl RegisterChurn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next register snapshot; returns the names that changed.
    pub fn observe(&mut self, current: &RegisterSet) -> Vec<String> {
        self.stops += 1;
        let changed = match &self.prev {
            Some(prev) if prev.thread_id == current.thread_id => current
                .diff(prev)
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        for name in &changed {
            *self.changes.entry(name.clone()).or_default() += 1;
        }
        self.prev = Some(current.clone());
        changed
    }

    /// Registers ordered by how frequently they changed (most first).
    pub fn ranked(&self) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self.changes.iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    pub fn stops(&self) -> u64 {
        self.stops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_core::{Register, RegisterRole, ThreadId};

    fn set(x0: u64, sp: u64) -> RegisterSet {
        RegisterSet {
            thread_id: ThreadId::new(1),
            registers: vec![
                Register::new("x0", x0, 64, RegisterRole::General),
                Register::new("sp", sp, 64, RegisterRole::StackPointer),
            ],
        }
    }

    #[test]
    fn tracks_change_frequency() {
        let mut churn = RegisterChurn::new();
        assert!(churn.observe(&set(0, 100)).is_empty()); // first: no baseline
        assert_eq!(churn.observe(&set(1, 100)), vec!["x0"]);
        assert_eq!(churn.observe(&set(2, 90)), vec!["x0", "sp"]);
        let ranked = churn.ranked();
        assert_eq!(ranked[0].0, "x0");
        assert_eq!(ranked[0].1, 2);
    }
}
