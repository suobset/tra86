//! Architecture-aware instruction classification.
//!
//! Ported and generalized from tra86, which was x86-only. Control-flow class is
//! derived from the mnemonic per architecture; callers use it as a *hint*, not
//! ground truth (the real transition signal in Bind comes from function-symbol
//! changes across the event stream).

use bind_core::Architecture;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlFlow {
    Linear,
    Call,
    Return,
    Jump,
    ConditionalBranch,
    Syscall,
    Unknown,
}

/// Classifies a mnemonic for the given architecture.
pub fn classify(arch: Architecture, mnemonic: &str) -> ControlFlow {
    let m = mnemonic.trim().to_ascii_lowercase();
    match arch {
        Architecture::X86_64 => classify_x86(&m),
        Architecture::Aarch64 => classify_arm64(&m),
        Architecture::Unknown => ControlFlow::Unknown,
    }
}

fn classify_x86(m: &str) -> ControlFlow {
    match m {
        "call" => ControlFlow::Call,
        "ret" | "retq" => ControlFlow::Return,
        "jmp" => ControlFlow::Jump,
        "syscall" | "sysenter" | "int" => ControlFlow::Syscall,
        _ if m.starts_with('j') => ControlFlow::ConditionalBranch,
        _ => ControlFlow::Linear,
    }
}

fn classify_arm64(m: &str) -> ControlFlow {
    match m {
        "bl" | "blr" => ControlFlow::Call,
        "ret" => ControlFlow::Return,
        "b" | "br" => ControlFlow::Jump,
        "svc" | "hvc" | "smc" => ControlFlow::Syscall,
        // `b.<cond>`, `cbz`, `cbnz`, `tbz`, `tbnz`.
        _ if m.starts_with("b.") || m.starts_with("cb") || m.starts_with("tb") => {
            ControlFlow::ConditionalBranch
        }
        _ => ControlFlow::Linear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x86_classification() {
        assert_eq!(classify(Architecture::X86_64, "call"), ControlFlow::Call);
        assert_eq!(classify(Architecture::X86_64, "ret"), ControlFlow::Return);
        assert_eq!(
            classify(Architecture::X86_64, "jne"),
            ControlFlow::ConditionalBranch
        );
        assert_eq!(classify(Architecture::X86_64, "mov"), ControlFlow::Linear);
    }

    #[test]
    fn arm64_classification() {
        assert_eq!(classify(Architecture::Aarch64, "bl"), ControlFlow::Call);
        assert_eq!(classify(Architecture::Aarch64, "ret"), ControlFlow::Return);
        assert_eq!(
            classify(Architecture::Aarch64, "b.lt"),
            ControlFlow::ConditionalBranch
        );
        assert_eq!(classify(Architecture::Aarch64, "svc"), ControlFlow::Syscall);
        assert_eq!(classify(Architecture::Aarch64, "add"), ControlFlow::Linear);
    }
}
