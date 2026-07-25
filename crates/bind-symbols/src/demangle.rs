//! Language-agnostic symbol demangling.
//!
//! This is the *only* place that knows about specific mangling schemes; general
//! UI and analysis code asks for a display name and gets a best-effort result,
//! never a hard-coded `if rust { ... } else if cpp { ... }`. Unknown or
//! already-plain symbols pass through unchanged.

/// Best-effort demangle of a linker symbol name.
///
/// Ordering: Rust v0 (`_R`) first, then Itanium C++ (`_Z` / `__Z`), then the
/// legacy Rust path, then identity. Returns an owned string equal to the input
/// when nothing applies.
pub fn demangle(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Rust v0 mangling.
    if trimmed.starts_with("_R") {
        if let Ok(d) = rustc_demangle::try_demangle(trimmed) {
            return format!("{d:#}");
        }
    }

    // Itanium C++ (also covers Swift's `_$s`? no — Swift handled as identity).
    if trimmed.starts_with("_Z") || trimmed.starts_with("__Z") {
        // Itanium names begin with `_Z`; macOS prefixes an extra leading
        // underscore (`__Z`), which must be dropped for the parser.
        let cpp_input = trimmed.strip_prefix("__").map(|r| {
            // re-add the single `_` that `_Z` requires
            format!("_{r}")
        });
        let cpp_input = cpp_input.as_deref().unwrap_or(trimmed);
        if let Ok(sym) = cpp_demangle::Symbol::new(cpp_input) {
            return sym.to_string();
        }
        // Fall back to legacy Rust which also historically used `_Z`.
        if let Ok(d) = rustc_demangle::try_demangle(trimmed) {
            return format!("{d:#}");
        }
    }

    // Legacy Rust names that don't start with `_Z` (rare) or a final attempt.
    if let Ok(d) = rustc_demangle::try_demangle(trimmed) {
        let out = format!("{d:#}");
        if out != trimmed {
            return out;
        }
    }

    trimmed.to_string()
}

/// True if `name` looks mangled (worth demangling / distinguishing in UI).
pub fn looks_mangled(name: &str) -> bool {
    let n = name.trim();
    n.starts_with("_R") || n.starts_with("_Z") || n.starts_with("__Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(demangle("main"), "main");
        assert_eq!(demangle("helper"), "helper");
        assert_eq!(demangle(""), "");
    }

    #[test]
    fn cpp_itanium_demangles() {
        // `_Z3fooi` == `foo(int)`
        assert_eq!(demangle("_Z3fooi"), "foo(int)");
    }

    #[test]
    fn rust_legacy_demangles() {
        // Legacy Rust symbol for `core::ptr::drop_in_place`-style names.
        let out = demangle("_ZN4core3fmt9Formatter3pad17h0123456789abcdefE");
        assert!(out.contains("core::fmt::Formatter::pad"), "got {out}");
    }

    #[test]
    fn mangled_detection() {
        assert!(looks_mangled("_Z3fooi"));
        assert!(!looks_mangled("main"));
    }
}
