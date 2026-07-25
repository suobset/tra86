//! `bind-trace`: bounded trace capture, a versioned on-disk format, and replay.
//!
//! - [`RingBuffer`] bounds interactive history and counts drops.
//! - [`TraceRecorder`] records events subject to a [`TraceFilter`], keeps a
//!   recent ring, and optionally persists a full [`TraceRecord`].
//! - [`read_trace`] / [`write_trace`] are the forwards-evolvable JSON codec.
//! - [`Replay`] walks a loaded trace with no live debugger.

pub mod filter;
pub mod record;
pub mod recorder;
pub mod ring;

pub use filter::TraceFilter;
pub use record::{
    sanitize_env, Annotation, TraceMetadata, TracePolicy, TraceRecord, SCHEMA_VERSION,
};
pub use recorder::{
    read_trace, read_trace_bytes, write_trace, Replay, TraceRecorder, DEFAULT_RING_CAPACITY,
};
pub use ring::RingBuffer;
