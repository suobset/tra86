//! Virtual address and address-range types.

use serde::{Deserialize, Serialize};

/// A target virtual address. A newtype so that address arithmetic and
/// formatting are explicit and cannot be accidentally mixed with sizes,
/// counts, or ids.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Address(pub u64);

impl Address {
    pub const NULL: Address = Address(0);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// Adds a byte offset, saturating rather than wrapping so a bad offset
    /// cannot silently wrap to a low address.
    pub fn offset(self, delta: u64) -> Address {
        Address(self.0.saturating_add(delta))
    }

    /// Signed distance `self - other`, useful for stack-pointer deltas.
    pub fn distance_from(self, other: Address) -> i64 {
        (self.0 as i128 - other.0 as i128) as i64
    }
}

impl core::fmt::Display for Address {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}

impl From<u64> for Address {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

/// A half-open range `[start, start+len)` of virtual addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressRange {
    pub start: Address,
    pub len: u64,
}

impl AddressRange {
    pub const fn new(start: Address, len: u64) -> Self {
        Self { start, len }
    }

    /// Constructs from an inclusive-start/exclusive-end pair. If `end` is below
    /// `start` the range is treated as empty.
    pub fn from_bounds(start: Address, end: Address) -> Self {
        let len = end.raw().saturating_sub(start.raw());
        Self { start, len }
    }

    pub fn end(self) -> Address {
        self.start.offset(self.len)
    }

    pub fn contains(self, addr: Address) -> bool {
        addr.raw() >= self.start.raw() && addr.raw() < self.end().raw()
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

impl core::fmt::Display for AddressRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}, {})", self.start, self.end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_saturates() {
        assert_eq!(Address(u64::MAX).offset(10), Address(u64::MAX));
        assert_eq!(Address(0x1000).offset(0x10), Address(0x1010));
    }

    #[test]
    fn distance_is_signed() {
        assert_eq!(Address(0x1010).distance_from(Address(0x1000)), 0x10);
        assert_eq!(Address(0x1000).distance_from(Address(0x1010)), -0x10);
    }

    #[test]
    fn range_contains_half_open() {
        let r = AddressRange::new(Address(0x1000), 0x20);
        assert!(r.contains(Address(0x1000)));
        assert!(r.contains(Address(0x101f)));
        assert!(!r.contains(Address(0x1020)));
        assert_eq!(r.end(), Address(0x1020));
    }

    #[test]
    fn from_bounds_handles_inverted() {
        let r = AddressRange::from_bounds(Address(0x2000), Address(0x1000));
        assert!(r.is_empty());
    }
}
