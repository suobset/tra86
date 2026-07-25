//! Trace recording, persistence, and replay.
//!
//! The recorder keeps a bounded in-memory ring of recent events for the
//! interactive timeline and, when a path is configured, also appends every
//! accepted event to a growing full record for persistence. Reads and replay
//! work purely off the serialized form with no debugger present.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bind_core::{BindError, BindResult, SequencedEvent};

use crate::filter::TraceFilter;
use crate::record::{TraceMetadata, TraceRecord};
use crate::ring::RingBuffer;

/// Default interactive retention: recent events kept in memory for the timeline.
pub const DEFAULT_RING_CAPACITY: usize = 50_000;

pub struct TraceRecorder {
    ring: RingBuffer<SequencedEvent>,
    filter: TraceFilter,
    /// When recording to disk, the accumulating full record + destination.
    persistent: Option<(TraceRecord, PathBuf)>,
    /// Events rejected by the filter (still counted for honesty).
    filtered_out: u64,
    metadata: TraceMetadata,
}

impl TraceRecorder {
    pub fn new(metadata: TraceMetadata, capacity: usize) -> Self {
        Self {
            ring: RingBuffer::new(capacity),
            filter: TraceFilter::accept_all(),
            persistent: None,
            filtered_out: 0,
            metadata,
        }
    }

    pub fn set_filter(&mut self, filter: TraceFilter) {
        self.filter = filter;
    }

    /// Mutable access to the session metadata (e.g. to fill in arch/executable
    /// once the target has loaded).
    pub fn metadata_mut(&mut self) -> &mut TraceMetadata {
        &mut self.metadata
    }

    /// Begins persisting to `path`. Existing recent events are seeded into the
    /// record so a mid-session start still captures buffered history.
    pub fn start_persisting(&mut self, path: impl Into<PathBuf>) {
        let mut record = TraceRecord::new(self.metadata.clone());
        record.events.extend(self.ring.iter().cloned());
        self.persistent = Some((record, path.into()));
    }

    /// Stops persisting and flushes the accumulated record to disk.
    pub fn stop_persisting(&mut self) -> BindResult<Option<PathBuf>> {
        if let Some((mut record, path)) = self.persistent.take() {
            record.metadata.dropped_events = self.ring.dropped() + self.filtered_out;
            record.metadata.ended_at = Some(chrono::Utc::now());
            write_trace(&record, &path)?;
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    pub fn is_persisting(&self) -> bool {
        self.persistent.is_some()
    }

    /// Records one event subject to the active filter.
    pub fn record(&mut self, event: SequencedEvent) {
        if !self.filter.accepts(&event) {
            self.filtered_out += 1;
            return;
        }
        if let Some((record, _)) = &mut self.persistent {
            record.events.push(event.clone());
        }
        self.ring.push(event);
    }

    pub fn recent(&self, n: usize) -> impl Iterator<Item = &SequencedEvent> {
        self.ring.recent(n)
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn total(&self) -> u64 {
        self.ring.total()
    }

    /// Total events not retained in memory (evicted + filtered out).
    pub fn dropped(&self) -> u64 {
        self.ring.dropped() + self.filtered_out
    }

    pub fn last(&self) -> Option<&SequencedEvent> {
        self.ring.last()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &SequencedEvent> {
        self.ring.iter()
    }
}

/// Writes a trace to `path` as pretty JSON (atomically via a temp file).
pub fn write_trace(record: &TraceRecord, path: impl AsRef<Path>) -> BindResult<()> {
    let path = path.as_ref();
    let json = serde_json::to_vec_pretty(record)
        .map_err(|e| BindError::Trace(format!("serialize trace: {e}")))?;
    let tmp = path.with_extension("bindtrace.tmp");
    write_all(&tmp, &json)
        .map_err(|e| BindError::Trace(format!("write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path)
        .map_err(|e| BindError::Trace(format!("finalize {}: {e}", path.display())))?;
    Ok(())
}

fn write_all(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.flush()
}

/// Reads a trace from `path`, tolerating unknown fields from newer writers but
/// refusing an incompatible major schema version.
pub fn read_trace(path: impl AsRef<Path>) -> BindResult<TraceRecord> {
    let path = path.as_ref();
    let bytes =
        fs::read(path).map_err(|e| BindError::Trace(format!("read {}: {e}", path.display())))?;
    read_trace_bytes(&bytes)
}

pub fn read_trace_bytes(bytes: &[u8]) -> BindResult<TraceRecord> {
    let record: TraceRecord =
        serde_json::from_slice(bytes).map_err(|e| BindError::Trace(format!("parse trace: {e}")))?;
    if !record.metadata.is_compatible() {
        return Err(BindError::Trace(format!(
            "trace schema version {} is newer than supported {}",
            record.metadata.schema_version,
            crate::record::SCHEMA_VERSION
        )));
    }
    Ok(record)
}

/// A replay cursor over a loaded trace. No live debugger required.
pub struct Replay {
    record: TraceRecord,
    pos: usize,
}

impl Replay {
    pub fn new(record: TraceRecord) -> Self {
        Self { record, pos: 0 }
    }

    pub fn from_path(path: impl AsRef<Path>) -> BindResult<Self> {
        Ok(Self::new(read_trace(path)?))
    }

    pub fn metadata(&self) -> &TraceMetadata {
        &self.record.metadata
    }

    pub fn len(&self) -> usize {
        self.record.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.record.events.is_empty()
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn reset(&mut self) {
        self.pos = 0;
    }

    /// Advances to and returns the next event.
    pub fn next_event(&mut self) -> Option<&SequencedEvent> {
        let ev = self.record.events.get(self.pos);
        if ev.is_some() {
            self.pos += 1;
        }
        ev
    }

    pub fn events(&self) -> &[SequencedEvent] {
        &self.record.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_core::{Architecture, DebugEvent, EventSeq, StopReason, ThreadId};

    fn ev(seq: u64) -> SequencedEvent {
        SequencedEvent::new(
            EventSeq::new(seq),
            seq,
            DebugEvent::Stopped {
                thread: ThreadId::new(1),
                reason: StopReason::Step,
                pc: None,
            },
        )
    }

    #[test]
    fn roundtrip_write_read() {
        let dir = std::env::temp_dir().join(format!("bindtrace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.bindtrace");

        let mut rec = TraceRecorder::new(TraceMetadata::new(Architecture::Aarch64), 100);
        rec.start_persisting(&path);
        for i in 0..10 {
            rec.record(ev(i));
        }
        let written = rec.stop_persisting().unwrap();
        assert_eq!(written.as_deref(), Some(path.as_path()));

        let loaded = read_trace(&path).unwrap();
        assert_eq!(loaded.events.len(), 10);
        assert_eq!(loaded.metadata.arch, Architecture::Aarch64);

        let mut replay = Replay::new(loaded);
        let mut count = 0;
        while replay.next_event().is_some() {
            count += 1;
        }
        assert_eq!(count, 10);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        // Simulate a trace written by a newer Bind that added a field.
        let json = r#"{
            "metadata": {
                "schema_version": 1,
                "bind_version": "9.9.9",
                "arch": "Aarch64",
                "future_field": {"nested": true}
            },
            "events": [],
            "another_future_field": 42
        }"#;
        let rec = read_trace_bytes(json.as_bytes()).unwrap();
        assert_eq!(rec.metadata.bind_version, "9.9.9");
    }

    #[test]
    fn incompatible_major_is_rejected() {
        let json = r#"{"metadata":{"schema_version":999,"bind_version":"1","arch":"Aarch64"},"events":[]}"#;
        assert!(read_trace_bytes(json.as_bytes()).is_err());
    }

    #[test]
    fn ring_bound_reports_drops() {
        let mut rec = TraceRecorder::new(TraceMetadata::new(Architecture::Unknown), 4);
        for i in 0..10 {
            rec.record(ev(i));
        }
        assert_eq!(rec.len(), 4);
        assert!(rec.dropped() >= 6);
        assert_eq!(rec.total(), 10);
    }
}
