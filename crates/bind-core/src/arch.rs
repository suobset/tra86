//! Architecture and language metadata.
//!
//! Architecture-specific knowledge (pointer width, canonical register roles,
//! the name of the program counter / stack pointer) is centralized here behind
//! an enum so the rest of the codebase never does ad-hoc string checks like
//! `if name == "rip"`. Backends report the architecture they observe; higher
//! layers ask this module questions about it.

use serde::{Deserialize, Serialize};

use crate::model::RegisterRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Architecture {
    X86_64,
    Aarch64,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Endianness {
    #[default]
    Little,
    Big,
}

impl Architecture {
    /// Best-effort parse of an LLDB / target triple architecture token.
    pub fn from_triple_arch(token: &str) -> Architecture {
        let t = token.trim().to_ascii_lowercase();
        if t.starts_with("x86_64") || t == "amd64" {
            Architecture::X86_64
        } else if t.starts_with("aarch64") || t.starts_with("arm64") {
            Architecture::Aarch64
        } else {
            Architecture::Unknown
        }
    }

    pub fn pointer_width_bits(self) -> u8 {
        match self {
            Architecture::X86_64 | Architecture::Aarch64 => 64,
            // Unknown: assume 64-bit, the only host class Bind currently targets.
            Architecture::Unknown => 64,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Architecture::X86_64 => "x86-64",
            Architecture::Aarch64 => "aarch64",
            Architecture::Unknown => "unknown",
        }
    }

    /// Canonical program-counter register name for this architecture, if known.
    pub fn program_counter_name(self) -> Option<&'static str> {
        match self {
            Architecture::X86_64 => Some("rip"),
            Architecture::Aarch64 => Some("pc"),
            Architecture::Unknown => None,
        }
    }

    /// Canonical stack-pointer register name for this architecture, if known.
    pub fn stack_pointer_name(self) -> Option<&'static str> {
        match self {
            Architecture::X86_64 => Some("rsp"),
            Architecture::Aarch64 => Some("sp"),
            Architecture::Unknown => None,
        }
    }

    /// Classifies a register purely from its name for this architecture. This
    /// is the *only* place that hard-codes register-name knowledge; backends
    /// should prefer authoritative role information when they have it and fall
    /// back to this when they do not.
    pub fn classify_register(self, name: &str) -> RegisterRole {
        let n = name.trim().to_ascii_lowercase();
        match self {
            Architecture::X86_64 => match n.as_str() {
                "rip" => RegisterRole::ProgramCounter,
                "rsp" => RegisterRole::StackPointer,
                "rbp" => RegisterRole::FramePointer,
                "rflags" | "eflags" | "flags" => RegisterRole::Flags,
                _ if n.starts_with('r') || n.starts_with('e') => RegisterRole::General,
                _ if n.starts_with("xmm") || n.starts_with("ymm") || n.starts_with("zmm") => {
                    RegisterRole::Vector
                }
                _ => RegisterRole::Other,
            },
            Architecture::Aarch64 => match n.as_str() {
                "pc" => RegisterRole::ProgramCounter,
                "sp" => RegisterRole::StackPointer,
                "fp" | "x29" => RegisterRole::FramePointer,
                "cpsr" | "pstate" => RegisterRole::Flags,
                "lr" | "x30" => RegisterRole::General,
                _ if n.starts_with('x') || n.starts_with('w') => RegisterRole::General,
                _ if n.starts_with('v') || n.starts_with('q') || n.starts_with('d') => {
                    RegisterRole::Vector
                }
                _ => RegisterRole::Other,
            },
            Architecture::Unknown => RegisterRole::Other,
        }
    }
}

/// Source language as reported by debug info. This is optional metadata layered
/// on top of the architecture-level model; Bind must function when it is
/// `Unknown` (stripped binaries, missing debug info, mixed-language processes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    C,
    Cpp,
    Rust,
    Swift,
    ObjectiveC,
    Zig,
    Assembly,
    Other,
    #[default]
    Unknown,
}

impl Language {
    /// Parse an LLDB language string (`"c++"`, `"rust"`, `"swift"`, ...).
    pub fn from_lldb(token: &str) -> Language {
        match token.trim().to_ascii_lowercase().as_str() {
            "c" | "c89" | "c99" | "c11" => Language::C,
            "c++" | "cpp" | "c++11" | "c++14" | "c++17" | "c++20" => Language::Cpp,
            "rust" => Language::Rust,
            "swift" => Language::Swift,
            "objective-c" | "objc" => Language::ObjectiveC,
            "zig" => Language::Zig,
            "asm" | "assembly" => Language::Assembly,
            "" | "unknown" => Language::Unknown,
            _ => Language::Other,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Rust => "Rust",
            Language::Swift => "Swift",
            Language::ObjectiveC => "Objective-C",
            Language::Zig => "Zig",
            Language::Assembly => "assembly",
            Language::Other => "other",
            Language::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_arch_parsing() {
        assert_eq!(
            Architecture::from_triple_arch("x86_64-pc-linux"),
            Architecture::X86_64
        );
        assert_eq!(
            Architecture::from_triple_arch("arm64"),
            Architecture::Aarch64
        );
        assert_eq!(
            Architecture::from_triple_arch("aarch64-apple-darwin"),
            Architecture::Aarch64
        );
        assert_eq!(
            Architecture::from_triple_arch("mips"),
            Architecture::Unknown
        );
    }

    #[test]
    fn register_roles_are_arch_specific() {
        assert_eq!(
            Architecture::X86_64.classify_register("rip"),
            RegisterRole::ProgramCounter
        );
        assert_eq!(
            Architecture::Aarch64.classify_register("pc"),
            RegisterRole::ProgramCounter
        );
        // "pc" is not special on x86-64.
        assert_ne!(
            Architecture::X86_64.classify_register("pc"),
            RegisterRole::ProgramCounter
        );
        assert_eq!(
            Architecture::Aarch64.classify_register("sp"),
            RegisterRole::StackPointer
        );
    }

    #[test]
    fn language_parsing() {
        assert_eq!(Language::from_lldb("c++"), Language::Cpp);
        assert_eq!(Language::from_lldb("Rust"), Language::Rust);
        assert_eq!(Language::from_lldb(""), Language::Unknown);
        assert_eq!(Language::from_lldb("fortran"), Language::Other);
    }
}
