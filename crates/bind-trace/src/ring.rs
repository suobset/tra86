//! A bounded ring buffer that counts what it drops.
//!
//! Instruction tracing can produce enormous volumes of events, so interactive
//! history is kept in a fixed-capacity buffer. Crucially the buffer *reports*
//! how many items it has evicted, so the UI can honestly say "showing last N of
//! M events" rather than silently pretending the trace is complete.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    items: VecDeque<T>,
    capacity: usize,
    dropped: u64,
    total: u64,
}

impl<T> RingBuffer<T> {
    /// Creates a ring buffer holding at most `capacity` items (min 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
            dropped: 0,
            total: 0,
        }
    }

    /// Pushes an item, evicting (and counting) the oldest if full.
    pub fn push(&mut self, item: T) {
        self.total += 1;
        if self.items.len() == self.capacity {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(item);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Total items ever pushed (retained + dropped).
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Items evicted because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.items.iter()
    }

    /// The most recent `n` items, oldest-first.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &T> {
        let skip = self.items.len().saturating_sub(n);
        self.items.iter().skip(skip)
    }

    pub fn last(&self) -> Option<&T> {
        self.items.back()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.dropped = 0;
        self.total = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_and_counts_drops() {
        let mut r = RingBuffer::new(3);
        for i in 0..5 {
            r.push(i);
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.dropped(), 2);
        assert_eq!(r.total(), 5);
        let items: Vec<_> = r.iter().copied().collect();
        assert_eq!(items, vec![2, 3, 4]);
    }

    #[test]
    fn recent_returns_tail() {
        let mut r = RingBuffer::new(10);
        for i in 0..5 {
            r.push(i);
        }
        let last2: Vec<_> = r.recent(2).copied().collect();
        assert_eq!(last2, vec![3, 4]);
    }

    #[test]
    fn zero_capacity_is_clamped() {
        let mut r = RingBuffer::new(0);
        r.push(1);
        assert_eq!(r.len(), 1);
    }
}
