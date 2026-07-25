//! Findings: what analyses emit.
//!
//! Every finding states its confidence and, when inferred, its limitations.
//! Bind does not claim that an inferred call, return, or hot region is certain
//! unless the underlying event data directly supports it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    /// Directly supported by the event stream (e.g. a stop-reason tally).
    High,
    /// Inferred but well-grounded (e.g. function transitions from stepping).
    Medium,
    /// Heuristic; treat as a hint (e.g. stack depth from inferred calls).
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Name of the analyzer that produced this.
    pub analyzer: String,
    /// Short machine-ish category (e.g. "hot-function", "stop-summary").
    pub kind: String,
    pub summary: String,
    /// Supporting facts, human-readable.
    pub evidence: Vec<String>,
    /// Related event sequence numbers, for timeline cross-linking.
    pub related_events: Vec<u64>,
    /// Relevant symbol / source / address, when applicable.
    pub location: Option<String>,
    pub confidence: Confidence,
    /// Honest statement of what this finding does *not* establish.
    pub limitations: Option<String>,
}

impl Finding {
    pub fn new(
        analyzer: impl Into<String>,
        kind: impl Into<String>,
        summary: impl Into<String>,
        confidence: Confidence,
    ) -> Self {
        Self {
            analyzer: analyzer.into(),
            kind: kind.into(),
            summary: summary.into(),
            evidence: Vec::new(),
            related_events: Vec::new(),
            location: None,
            confidence,
            limitations: None,
        }
    }

    pub fn with_evidence(mut self, e: impl Into<String>) -> Self {
        self.evidence.push(e.into());
        self
    }

    pub fn with_location(mut self, loc: impl Into<String>) -> Self {
        self.location = Some(loc.into());
        self
    }

    pub fn with_limitations(mut self, l: impl Into<String>) -> Self {
        self.limitations = Some(l.into());
        self
    }
}
