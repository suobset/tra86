//! The versioned, forwards-evolvable trace format.
//!
//! A trace is self-describing metadata plus an ordered event stream. It is
//! deliberately plain serde/JSON of `bind-core` types — never a serialized
//! backend object graph — so a trace is replayable with no live debugger and no
//! LLDB.
//!
//! Forwards evolution: readers do **not** use `deny_unknown_fields`, and every
//! added field carries `#[serde(default)]`, so a newer trace opened by an older
//! Bind ignores unknown fields instead of failing catastrophically. The
//! `schema_version` lets readers refuse to *misinterpret* an incompatible major
//! version.

use bind_core::{Architecture, SequencedEvent};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current trace schema version. Bump the major on breaking layout changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Sampling / filtering policy that produced a trace, recorded for honesty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TracePolicy {
    /// Human-readable description of any filter applied.
    #[serde(default)]
    pub filter_description: Option<String>,
    /// 1 = every event; N = roughly one in N (0/absent means unsampled).
    #[serde(default)]
    pub sample_rate: u32,
    /// Ring-buffer capacity used for in-memory retention, if bounded.
    #[serde(default)]
    pub retention_capacity: Option<usize>,
}

/// Metadata describing the environment and target a trace was captured against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceMetadata {
    pub schema_version: u32,
    pub bind_version: String,
    #[serde(default)]
    pub lldb_version: Option<String>,
    #[serde(default)]
    pub host_os: String,
    pub arch: Architecture,
    #[serde(default)]
    pub executable: Option<String>,
    /// Content hash of the executable, when Bind could compute one.
    #[serde(default)]
    pub executable_hash: Option<String>,
    #[serde(default)]
    pub command_line: Vec<String>,
    /// Environment metadata with secrets excluded (see [`sanitize_env`]).
    #[serde(default)]
    pub environment: Vec<(String, String)>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub policy: TracePolicy,
    /// Count of events dropped by retention/sampling, surfaced honestly.
    #[serde(default)]
    pub dropped_events: u64,
}

impl TraceMetadata {
    pub fn new(arch: Architecture) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            bind_version: env!("CARGO_PKG_VERSION").to_string(),
            lldb_version: None,
            host_os: std::env::consts::OS.to_string(),
            arch,
            executable: None,
            executable_hash: None,
            command_line: Vec::new(),
            environment: Vec::new(),
            modules: Vec::new(),
            started_at: Some(Utc::now()),
            ended_at: None,
            policy: TracePolicy::default(),
            dropped_events: 0,
        }
    }

    /// Whether this reader can faithfully interpret the trace's major version.
    pub fn is_compatible(&self) -> bool {
        self.schema_version <= SCHEMA_VERSION
    }
}

/// User annotations attached to a trace (notes, bookmarks) keyed loosely so the
/// format can grow without a schema break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Annotation {
    pub seq: u64,
    pub note: String,
}

/// A full trace: metadata plus the recorded, sequenced event stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRecord {
    pub metadata: TraceMetadata,
    #[serde(default)]
    pub events: Vec<SequencedEvent>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl TraceRecord {
    pub fn new(metadata: TraceMetadata) -> Self {
        Self {
            metadata,
            events: Vec::new(),
            annotations: Vec::new(),
        }
    }
}

/// Drops environment variables whose names look secret. Bind never persists
/// these into a trace by accident.
pub fn sanitize_env(env: &[(String, String)]) -> Vec<(String, String)> {
    const DENY: &[&str] = &[
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "KEY",
        "CREDENTIAL",
        "AUTH",
        "SESSION",
    ];
    env.iter()
        .filter(|(k, _)| {
            let up = k.to_ascii_uppercase();
            !DENY.iter().any(|d| up.contains(d))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_drops_secrets() {
        let env = vec![
            ("PATH".into(), "/bin".into()),
            ("API_TOKEN".into(), "abc".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "xyz".into()),
            ("HOME".into(), "/home/x".into()),
        ];
        let clean = sanitize_env(&env);
        let keys: Vec<_> = clean.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"HOME"));
        assert!(!keys.contains(&"API_TOKEN"));
        assert!(!keys.contains(&"AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn compatibility_check() {
        let mut m = TraceMetadata::new(Architecture::Aarch64);
        assert!(m.is_compatible());
        m.schema_version = SCHEMA_VERSION + 1;
        assert!(!m.is_compatible());
    }
}
