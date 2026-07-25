//! Stable, type-distinct identifiers.
//!
//! These are deliberately newtypes rather than bare integers so that a thread
//! id can never be passed where a frame id is expected. Ids are allocated by
//! whichever layer owns the corresponding concept (the debugger worker for
//! session/process ids, backends for thread/frame/breakpoint ids, etc.).

use serde::{Deserialize, Serialize};

macro_rules! newtype_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<u64> for $name {
            fn from(raw: u64) -> Self {
                Self(raw)
            }
        }
    };
}

newtype_id!(
    /// Identifies one Bind debugging session (one target under control).
    SessionId
);
newtype_id!(
    /// Debugger-assigned thread identity. Not necessarily the OS tid, but
    /// stable for the lifetime of the thread within a session.
    ThreadId
);
newtype_id!(
    /// Frame identity within a thread's unwound stack at a given stop.
    FrameId
);
newtype_id!(
    /// Breakpoint identity, stable across resolution of multiple locations.
    BreakpointId
);
newtype_id!(
    /// Watchpoint identity.
    WatchpointId
);
newtype_id!(
    /// Loaded module (executable or shared library) identity.
    ModuleId
);
newtype_id!(
    /// Monotonic sequence number for the normalized event stream.
    EventSeq
);

impl EventSeq {
    /// The sequence number that precedes any real event.
    pub const ZERO: EventSeq = EventSeq(0);

    /// Returns the next sequence number, saturating at the maximum.
    pub fn next(self) -> EventSeq {
        EventSeq(self.0.saturating_add(1))
    }
}

/// Operating-system process id. Kept distinct from the debugger's internal ids
/// because the OS pid is an external, reused value with different semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pid(pub u32);

impl Pid {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl core::fmt::Display for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_type_distinct_but_share_representation() {
        let t = ThreadId::new(7);
        let f = FrameId::new(7);
        assert_eq!(t.raw(), f.raw());
        // The following would not compile, which is the point:
        // let _: ThreadId = f;
    }

    #[test]
    fn event_seq_advances_and_saturates() {
        assert_eq!(EventSeq::ZERO.next(), EventSeq(1));
        assert_eq!(EventSeq(u64::MAX).next(), EventSeq(u64::MAX));
    }
}
