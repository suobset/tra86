//! Selective tracing filters.
//!
//! Filters decide which events are retained/recorded. They compose: an event is
//! accepted if it passes every active predicate. Filtering by event kind is
//! always available; address/thread filtering applies where the event carries
//! that context.

use bind_core::{AddressRange, DebugEvent, SequencedEvent, ThreadId};

#[derive(Debug, Clone, Default)]
pub struct TraceFilter {
    /// If non-empty, only these event kinds are accepted.
    pub kinds: Vec<String>,
    /// If set, only events on this thread (events without a thread pass).
    pub thread: Option<ThreadId>,
    /// If set, only events whose address falls in this range pass.
    pub address_range: Option<AddressRange>,
}

impl TraceFilter {
    pub fn accept_all() -> Self {
        Self::default()
    }

    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.kinds = kinds.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_thread(mut self, thread: ThreadId) -> Self {
        self.thread = Some(thread);
        self
    }

    pub fn with_range(mut self, range: AddressRange) -> Self {
        self.address_range = Some(range);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty() && self.thread.is_none() && self.address_range.is_none()
    }

    /// Returns the address a filter should test for this event, if any.
    fn event_address(event: &DebugEvent) -> Option<bind_core::Address> {
        match event {
            DebugEvent::Stopped { pc, .. } => *pc,
            DebugEvent::InstructionStepped { to, .. } => Some(*to),
            DebugEvent::BreakpointResolved { address, .. } => Some(*address),
            _ => None,
        }
    }

    pub fn accepts(&self, ev: &SequencedEvent) -> bool {
        if !self.kinds.is_empty() && !self.kinds.iter().any(|k| k == ev.event.kind()) {
            return false;
        }
        if let Some(want) = self.thread {
            if let Some(t) = ev.event.thread() {
                if t != want {
                    return false;
                }
            }
        }
        if let Some(range) = self.address_range {
            if let Some(addr) = Self::event_address(&ev.event) {
                if !range.contains(addr) {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_core::{Address, EventSeq, StopReason};

    fn stopped(thread: u64, pc: u64) -> SequencedEvent {
        SequencedEvent::new(
            EventSeq::new(1),
            0,
            DebugEvent::Stopped {
                thread: ThreadId::new(thread),
                reason: StopReason::Step,
                pc: Some(Address::new(pc)),
            },
        )
    }

    #[test]
    fn kind_filter() {
        let f = TraceFilter::accept_all().with_kinds(["continued"]);
        assert!(!f.accepts(&stopped(1, 0x1000)));
        let cont = SequencedEvent::new(EventSeq::new(1), 0, DebugEvent::Continued);
        assert!(f.accepts(&cont));
    }

    #[test]
    fn thread_and_range_filter() {
        let f = TraceFilter::accept_all()
            .with_thread(ThreadId::new(1))
            .with_range(AddressRange::new(Address::new(0x1000), 0x100));
        assert!(f.accepts(&stopped(1, 0x1010)));
        assert!(!f.accepts(&stopped(2, 0x1010))); // wrong thread
        assert!(!f.accepts(&stopped(1, 0x2000))); // out of range
    }
}
