//! Address → symbol / source resolution with caching.
//!
//! The index stores function symbols by their address range and answers
//! nearest-symbol queries (returning the containing symbol plus the offset of
//! the queried address). It degrades gracefully: a miss returns `None` rather
//! than an error, and demangling is memoized.

use std::collections::HashMap;

use bind_core::{Address, Symbol};

use crate::demangle::demangle;

#[derive(Debug, Clone)]
struct Entry {
    start: Address,
    /// Size in bytes; 0 means "unknown extent" (matched only exactly / as the
    /// nearest preceding symbol).
    size: u64,
    name: String,
    module: Option<String>,
}

/// An index of function symbols supporting nearest-address lookup.
#[derive(Debug, Default)]
pub struct SymbolIndex {
    entries: Vec<Entry>,
    /// Kept sorted by `start` lazily; `dirty` marks when a re-sort is needed.
    dirty: bool,
    by_name: HashMap<String, Address>,
    demangle_cache: HashMap<String, String>,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds a symbol. `size` may be 0 when unknown.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        start: Address,
        size: u64,
        module: Option<String>,
    ) {
        let name = name.into();
        self.by_name.entry(name.clone()).or_insert(start);
        self.entries.push(Entry {
            start,
            size,
            name,
            module,
        });
        self.dirty = true;
    }

    fn ensure_sorted(&mut self) {
        if self.dirty {
            self.entries.sort_by_key(|e| e.start.raw());
            self.dirty = false;
        }
    }

    /// Memoized demangling.
    pub fn display_name(&mut self, mangled: &str) -> String {
        if let Some(hit) = self.demangle_cache.get(mangled) {
            return hit.clone();
        }
        let d = demangle(mangled);
        self.demangle_cache.insert(mangled.to_string(), d.clone());
        d
    }

    /// Resolves an address to the containing (or nearest preceding) symbol.
    pub fn resolve(&mut self, addr: Address) -> Option<Symbol> {
        self.ensure_sorted();
        if self.entries.is_empty() {
            return None;
        }
        // Binary search for the last entry with start <= addr.
        let idx = match self
            .entries
            .binary_search_by_key(&addr.raw(), |e| e.start.raw())
        {
            Ok(i) => i,
            Err(0) => return None, // addr precedes all symbols
            Err(i) => i - 1,
        };
        let entry = self.entries[idx].clone();
        // If the entry has a known size, ensure the address is within it.
        if entry.size > 0 && addr.raw() >= entry.start.raw() + entry.size {
            return None;
        }
        let offset = addr.raw() - entry.start.raw();
        let demangled = {
            let d = self.display_name(&entry.name);
            if d != entry.name {
                Some(d)
            } else {
                None
            }
        };
        Some(Symbol {
            name: entry.name,
            demangled,
            module: entry.module,
            start: Some(entry.start),
            offset,
        })
    }

    /// The best display name for an address (demangled symbol + offset), if any.
    pub fn function_at(&mut self, addr: Address) -> Option<String> {
        self.resolve(addr).map(|s| {
            if s.offset == 0 {
                s.display().to_string()
            } else {
                format!("{}+0x{:x}", s.display(), s.offset)
            }
        })
    }

    pub fn address_of(&self, name: &str) -> Option<Address> {
        self.by_name.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> SymbolIndex {
        let mut idx = SymbolIndex::new();
        idx.insert("main", Address::new(0x1000), 0x20, Some("a.out".into()));
        idx.insert("helper", Address::new(0x1020), 0x10, Some("a.out".into()));
        idx.insert("_Z3fooi", Address::new(0x1040), 0x10, None);
        idx
    }

    #[test]
    fn resolves_within_range_with_offset() {
        let mut idx = index();
        let s = idx.resolve(Address::new(0x1008)).unwrap();
        assert_eq!(s.name, "main");
        assert_eq!(s.offset, 8);
    }

    #[test]
    fn resolves_and_demangles() {
        let mut idx = index();
        let s = idx.resolve(Address::new(0x1040)).unwrap();
        assert_eq!(s.display(), "foo(int)");
    }

    #[test]
    fn miss_below_all_symbols() {
        let mut idx = index();
        assert!(idx.resolve(Address::new(0x100)).is_none());
    }

    #[test]
    fn miss_past_known_size() {
        let mut idx = index();
        // helper is [0x1020, 0x1030); 0x1035 is past it and before foo.
        // Nearest-preceding is helper, but size bound rejects it.
        assert!(idx.resolve(Address::new(0x1035)).is_none());
    }

    #[test]
    fn function_at_formats_offset() {
        let mut idx = index();
        assert_eq!(
            idx.function_at(Address::new(0x1000)).as_deref(),
            Some("main")
        );
        assert_eq!(
            idx.function_at(Address::new(0x1004)).as_deref(),
            Some("main+0x4")
        );
    }
}
