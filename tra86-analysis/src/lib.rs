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
