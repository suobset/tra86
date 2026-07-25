//! `bind-symbols`: symbol/source resolution and demangling.
//!
//! Owns address→symbol lookup ([`SymbolIndex`]) and language-agnostic
//! [`demangle`]. Demangling is centralized here so no UI or analysis code
//! hard-codes Rust or C++ knowledge, and lookups degrade to `None` rather than
//! erroring when symbols are missing (stripped binaries, JIT code, etc.).

pub mod demangle;
pub mod index;

pub use demangle::{demangle, looks_mangled};
pub use index::SymbolIndex;
