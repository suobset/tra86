use tra86_core::{
    ControlFlowChange, ExecutionDelta, ExecutionHint, MemoryWrite, RegisterBank, RegisterDiff,
};

pub fn compute_register_delta(
    current: &RegisterBank,
    previous: Option<&RegisterBank>,
) -> Vec<RegisterDiff> {
    match previous {
        Some(prev) => current.diff(prev),
        None => Vec::new(),
    }
}

pub fn classify_mnemonic(mnemonic: &str) -> (ControlFlowChange, Vec<ExecutionHint>) {
    let lowered = mnemonic.to_ascii_lowercase();
    if lowered == "call" {
        return (ControlFlowChange::Call, vec![ExecutionHint::Call]);
    }
    if lowered == "ret" {
        return (
            ControlFlowChange::Return,
            vec![ExecutionHint::Return, ExecutionHint::Epilogue],
        );
    }
    if lowered.starts_with('j') {
        let hint = if lowered == "jmp" {
            ExecutionHint::Jump
        } else {
            ExecutionHint::ConditionalBranch
        };
        return (ControlFlowChange::Jump, vec![hint]);
    }
    if lowered == "syscall" || lowered == "svc" {
        return (ControlFlowChange::Syscall, vec![ExecutionHint::Syscall]);
    }

    let mut hints = Vec::new();
    if lowered == "push" {
        hints.push(ExecutionHint::Prologue);
    }
    if lowered == "leave" {
        hints.push(ExecutionHint::Epilogue);
    }

    (ControlFlowChange::Linear, hints)
}

pub fn build_delta(
    mnemonic: &str,
    changed_registers: Vec<RegisterDiff>,
    stack_pointer_delta: i64,
    memory_writes: Vec<MemoryWrite>,
) -> ExecutionDelta {
    let (control_flow, hints) = classify_mnemonic(mnemonic);
    ExecutionDelta {
        changed_registers,
        control_flow,
        stack_pointer_delta,
        memory_writes,
        hints,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tra86_core::{RegisterRole, RegisterValue};

    fn bank(values: &[(&str, &str)]) -> RegisterBank {
        RegisterBank {
            thread_id: 1,
            bank_name: "general".to_string(),
            registers: values
                .iter()
                .map(|(name, value)| RegisterValue {
                    name: (*name).to_string(),
                    value: (*value).to_string(),
                    size_bits: 64,
                    role: RegisterRole::General,
                })
                .collect(),
        }
    }

    #[test]
    fn compute_register_delta_reports_changed_registers() {
        let previous = bank(&[("rax", "0x1"), ("rbx", "0x2")]);
        let current = bank(&[("rax", "0x3"), ("rbx", "0x2")]);

        let diff = compute_register_delta(&current, Some(&previous));
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].name, "rax");
        assert_eq!(diff[0].old_value, "0x1");
        assert_eq!(diff[0].new_value, "0x3");
    }

    #[test]
    fn classify_mnemonic_detects_control_flow() {
        assert_eq!(classify_mnemonic("call").0, ControlFlowChange::Call);
        assert_eq!(classify_mnemonic("ret").0, ControlFlowChange::Return);
        assert_eq!(classify_mnemonic("jne").0, ControlFlowChange::Jump);
        assert_eq!(classify_mnemonic("syscall").0, ControlFlowChange::Syscall);
        assert_eq!(classify_mnemonic("mov").0, ControlFlowChange::Linear);
    }

    #[test]
    fn build_delta_carries_derived_hints() {
        let delta = build_delta("ret", Vec::new(), -8, Vec::new());
        assert_eq!(delta.control_flow, ControlFlowChange::Return);
        assert_eq!(delta.stack_pointer_delta, -8);
        assert!(delta.hints.contains(&ExecutionHint::Return));
        assert!(delta.hints.contains(&ExecutionHint::Epilogue));
    }
}
