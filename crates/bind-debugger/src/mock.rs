//! A fully in-memory backend.
//!
//! The mock backend simulates a tiny deterministic aarch64 program so the TUI,
//! analyses, trace recorder, and worker can all be developed and tested with no
//! LLDB, no compiler, and no target process. It is the backbone of Bind's
//! deterministic test suite (acceptance criterion: "the core and TUI can run
//! against a mock backend").
//!
//! The simulated program:
//! ```text
//! 0x1000  main:    mov   x0, #0        ; loop counter
//! 0x1004           bl    helper        ; call
//! 0x1008           add   x0, x0, #1
//! 0x100c           cmp   x0, #3
//! 0x1010           b.lt  0x1004        ; loop back to the call
//! 0x1014           ret
//! 0x1018  helper:  mov   x1, #42
//! 0x101c           ret
//! ```

use std::collections::VecDeque;

use bind_core::{
    Address, AttachSpec, Breakpoint, BreakpointId, BreakpointLocation, Capabilities, DebugEvent,
    Frame, Instruction, MemoryRegion, Module, ProcessInfo, ProcessLifecycle, Register,
    RegisterRole, RegisterSet, ResolvedLocation, RunTarget, StepKind, StopReason, Symbol, ThreadId,
    ThreadInfo, ThreadRunState,
};
use bind_core::{Architecture, BindError, BindResult, ModuleId, Pid};

use crate::backend::DebugBackend;

const BASE: u64 = 0x1000;

struct MockInsn {
    mnemonic: &'static str,
    operands: &'static str,
    bytes: [u8; 4],
    /// Index of the next instruction to execute (control flow). `None` = exit.
    next: usize,
    symbol: &'static str,
    line: u32,
}

fn program() -> Vec<MockInsn> {
    // `next` values encode: the loop branch at index 4 jumps back to index 1
    // (the call) until the counter check, then falls through to ret.
    vec![
        MockInsn {
            mnemonic: "mov",
            operands: "x0, #0",
            bytes: [0x00, 0x00, 0x80, 0xd2],
            next: 1,
            symbol: "main",
            line: 3,
        },
        // `bl` jumps to helper (index 6); the return address (index 2) is
        // pushed onto the call stack by `advance`.
        MockInsn {
            mnemonic: "bl",
            operands: "helper",
            bytes: [0x05, 0x00, 0x00, 0x94],
            next: 6,
            symbol: "main",
            line: 4,
        },
        MockInsn {
            mnemonic: "add",
            operands: "x0, x0, #1",
            bytes: [0x00, 0x04, 0x00, 0x91],
            next: 3,
            symbol: "main",
            line: 5,
        },
        MockInsn {
            mnemonic: "cmp",
            operands: "x0, #3",
            bytes: [0x1f, 0x0c, 0x00, 0xf1],
            next: 4,
            symbol: "main",
            line: 6,
        },
        // `b.lt` is resolved conditionally in `advance` (taken while x0 < 3).
        MockInsn {
            mnemonic: "b.lt",
            operands: "0x1004",
            bytes: [0x8b, 0xff, 0xff, 0x54],
            next: 1,
            symbol: "main",
            line: 6,
        },
        MockInsn {
            mnemonic: "ret",
            operands: "",
            bytes: [0xc0, 0x03, 0x5f, 0xd6],
            next: usize::MAX,
            symbol: "main",
            line: 7,
        },
        MockInsn {
            mnemonic: "mov",
            operands: "x1, #42",
            bytes: [0x41, 0x05, 0x80, 0xd2],
            next: 7,
            symbol: "helper",
            line: 11,
        },
        MockInsn {
            mnemonic: "ret",
            operands: "",
            bytes: [0xc0, 0x03, 0x5f, 0xd6],
            next: usize::MAX,
            symbol: "helper",
            line: 12,
        },
    ]
}

/// Address of instruction at slice index `i`.
fn addr_of(i: usize) -> Address {
    Address::new(BASE + (i as u64) * 4)
}

/// Slice index for a given address, if it lands on an instruction boundary.
fn index_of(addr: Address) -> Option<usize> {
    let raw = addr.raw();
    if raw < BASE {
        return None;
    }
    let off = raw - BASE;
    if !off.is_multiple_of(4) {
        return None;
    }
    let idx = (off / 4) as usize;
    if idx < program().len() {
        Some(idx)
    } else {
        None
    }
}

pub struct MockBackend {
    program: Vec<MockInsn>,
    lifecycle: ProcessLifecycle,
    pc_index: usize,
    /// Simulated x0 loop counter and stack pointer, so register diffs are real.
    x0: u64,
    sp: u64,
    /// Return-address stack for `bl`/`ret`, to synthesize frames honestly.
    call_stack: Vec<usize>,
    steps: u64,
    breakpoints: Vec<Breakpoint>,
    next_bp: u64,
    events: VecDeque<DebugEvent>,
    exit_code: Option<i32>,
    thread: ThreadId,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            program: program(),
            lifecycle: ProcessLifecycle::Idle,
            pc_index: 0,
            x0: 0,
            sp: 0x7fff_0000,
            call_stack: Vec::new(),
            steps: 0,
            breakpoints: Vec::new(),
            next_bp: 1,
            events: VecDeque::new(),
            exit_code: None,
            thread: ThreadId::new(1),
        }
    }
}

impl MockBackend {
    fn require_stopped(&self) -> BindResult<()> {
        if self.lifecycle.is_stopped() {
            Ok(())
        } else {
            Err(BindError::TargetState(format!(
                "operation requires a stopped process, but process is {:?}",
                self.lifecycle
            )))
        }
    }

    fn symbol_for(&self, i: usize) -> Symbol {
        let name = self.program[i].symbol;
        let start_index = self
            .program
            .iter()
            .position(|ins| ins.symbol == name)
            .unwrap_or(i);
        Symbol {
            name: name.to_string(),
            demangled: None,
            module: Some("mock".to_string()),
            start: Some(addr_of(start_index)),
            offset: (i - start_index) as u64 * 4,
        }
    }

    fn breakpoint_index_at(&self, i: usize) -> Option<BreakpointId> {
        let a = addr_of(i);
        self.breakpoints
            .iter()
            .find(|b| b.enabled && b.resolved.iter().any(|r| r.address == a))
            .map(|b| b.id)
    }

    /// Advance one instruction, updating simulated registers/stack. Returns the
    /// breakpoint hit at the *new* pc, if any.
    fn advance(&mut self) -> Option<BreakpointId> {
        let cur = self.pc_index;
        let insn = &self.program[cur];
        let mut next = insn.next;
        match insn.mnemonic {
            "bl" => {
                self.call_stack.push(cur + 1);
                self.sp -= 16;
            }
            "ret" => {
                self.sp += 16;
            }
            "add" => {
                self.x0 += 1;
            }
            "b.lt" => {
                // Conditional branch: taken while the loop counter is below the
                // bound, otherwise fall through to the following instruction.
                next = if self.x0 < 3 { insn.next } else { cur + 1 };
            }
            _ => {}
        }
        if next == usize::MAX {
            // `ret` from main ends the program; `ret` from helper returns.
            if let Some(ret_to) = self.call_stack.pop() {
                self.pc_index = ret_to;
            } else {
                self.lifecycle = ProcessLifecycle::Exited;
                self.exit_code = Some(0);
                return None;
            }
        } else {
            self.pc_index = next;
        }
        self.steps += 1;
        self.breakpoint_index_at(self.pc_index)
    }
}

impl DebugBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            launch: true,
            attach: false,
            core_files: false,
            detach: true,
            async_pause: false,
            step_instruction: true,
            step_source: true,
            breakpoints: true,
            conditional_breakpoints: false,
            watchpoints: false,
            read_memory: true,
            write_memory: false,
            disassemble: true,
            registers: true,
            jit_events: false,
            symbol_resolution: true,
            source_resolution: true,
        }
    }

    fn attach(&mut self, spec: &AttachSpec) -> BindResult<()> {
        match spec {
            AttachSpec::Launch(target) => {
                self.lifecycle = ProcessLifecycle::Stopped;
                self.pc_index = 0;
                self.events.push_back(DebugEvent::TargetLoaded {
                    program: target.program.clone(),
                    arch: Architecture::Aarch64,
                });
                self.events.push_back(DebugEvent::ProcessLaunched {
                    pid: Pid::new(4242),
                });
                self.events.push_back(DebugEvent::ThreadCreated {
                    thread: self.thread,
                });
                self.events.push_back(DebugEvent::Stopped {
                    thread: self.thread,
                    reason: StopReason::Signal("SIGSTOP".into()),
                    pc: Some(addr_of(self.pc_index)),
                });
                Ok(())
            }
            AttachSpec::Pid(_) => Err(BindError::Unsupported(
                "mock backend cannot attach to a pid".into(),
            )),
            AttachSpec::CoreFile { .. } => Err(BindError::Unsupported(
                "mock backend cannot open core files".into(),
            )),
        }
    }

    fn detach(&mut self) -> BindResult<()> {
        self.lifecycle = ProcessLifecycle::Detached;
        self.events.push_back(DebugEvent::ProcessDetached);
        Ok(())
    }

    fn terminate(&mut self) -> BindResult<()> {
        self.lifecycle = ProcessLifecycle::Exited;
        self.exit_code = Some(0);
        self.events.push_back(DebugEvent::ProcessExited { code: 0 });
        Ok(())
    }

    fn resume(&mut self) -> BindResult<()> {
        self.require_stopped()?;
        self.events.push_back(DebugEvent::Continued);
        // Run until a breakpoint or program exit, bounded to avoid runaway.
        for _ in 0..10_000 {
            match self.advance() {
                Some(bp) => {
                    self.lifecycle = ProcessLifecycle::Stopped;
                    self.events.push_back(DebugEvent::BreakpointHit {
                        breakpoint: bp,
                        thread: self.thread,
                    });
                    self.events.push_back(DebugEvent::Stopped {
                        thread: self.thread,
                        reason: StopReason::Breakpoint(bp),
                        pc: Some(addr_of(self.pc_index)),
                    });
                    if let Some(b) = self.breakpoints.iter_mut().find(|b| b.id == bp) {
                        b.hit_count += 1;
                    }
                    return Ok(());
                }
                None => {
                    if self.lifecycle == ProcessLifecycle::Exited {
                        self.events.push_back(DebugEvent::ProcessExited { code: 0 });
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    fn pause(&mut self) -> BindResult<()> {
        Err(BindError::CapabilityLimited(
            "mock backend runs synchronously; nothing to pause".into(),
        ))
    }

    fn step(&mut self, kind: StepKind) -> BindResult<()> {
        self.require_stopped()?;
        let from = Some(addr_of(self.pc_index));
        match kind {
            StepKind::Out => {
                // Run to the pending return address if inside a call.
                if self.call_stack.is_empty() {
                    self.advance();
                } else {
                    let target = *self.call_stack.last().unwrap();
                    for _ in 0..10_000 {
                        self.advance();
                        if self.pc_index == target || self.lifecycle == ProcessLifecycle::Exited {
                            break;
                        }
                    }
                }
            }
            _ => {
                self.advance();
            }
        }
        if self.lifecycle == ProcessLifecycle::Exited {
            self.events.push_back(DebugEvent::ProcessExited { code: 0 });
            return Ok(());
        }
        self.events.push_back(DebugEvent::InstructionStepped {
            thread: self.thread,
            from,
            to: addr_of(self.pc_index),
        });
        self.events.push_back(DebugEvent::Stopped {
            thread: self.thread,
            reason: StopReason::Step,
            pc: Some(addr_of(self.pc_index)),
        });
        Ok(())
    }

    fn run_to(&mut self, target: &RunTarget) -> BindResult<()> {
        self.require_stopped()?;
        let goal = match target {
            RunTarget::Address(a) => index_of(*a),
            RunTarget::Source { line, .. } => self.program.iter().position(|ins| ins.line == *line),
        };
        let Some(goal) = goal else {
            return Err(BindError::UserInput("no code at run-to target".into()));
        };
        for _ in 0..10_000 {
            self.advance();
            if self.pc_index == goal || self.lifecycle == ProcessLifecycle::Exited {
                break;
            }
        }
        self.events.push_back(DebugEvent::Stopped {
            thread: self.thread,
            reason: StopReason::Step,
            pc: Some(addr_of(self.pc_index)),
        });
        Ok(())
    }

    fn select_thread(&mut self, _thread: ThreadId) -> BindResult<()> {
        Ok(())
    }

    fn select_frame(&mut self, _frame_index: usize) -> BindResult<()> {
        Ok(())
    }

    fn add_breakpoint(
        &mut self,
        location: &BreakpointLocation,
        condition: Option<&str>,
    ) -> BindResult<Breakpoint> {
        let resolved_index = match location {
            BreakpointLocation::Address(a) => index_of(*a),
            BreakpointLocation::Symbol(name) => {
                self.program.iter().position(|ins| ins.symbol == name)
            }
            BreakpointLocation::Source { line, .. } => {
                self.program.iter().position(|ins| ins.line == *line)
            }
        };
        let id = BreakpointId::new(self.next_bp);
        self.next_bp += 1;
        let resolved = resolved_index
            .map(|i| {
                vec![ResolvedLocation {
                    address: addr_of(i),
                    symbol: Some(self.symbol_for(i)),
                    source: None,
                }]
            })
            .unwrap_or_default();
        let bp = Breakpoint {
            id,
            location: location.clone(),
            enabled: true,
            hit_count: 0,
            condition: condition.map(str::to_string),
            resolved,
        };
        if let Some(i) = resolved_index {
            self.events.push_back(DebugEvent::BreakpointResolved {
                breakpoint: id,
                address: addr_of(i),
            });
        }
        self.breakpoints.push(bp.clone());
        Ok(bp)
    }

    fn remove_breakpoint(&mut self, id: BreakpointId) -> BindResult<()> {
        self.breakpoints.retain(|b| b.id != id);
        Ok(())
    }

    fn enable_breakpoint(&mut self, id: BreakpointId, enabled: bool) -> BindResult<()> {
        if let Some(b) = self.breakpoints.iter_mut().find(|b| b.id == id) {
            b.enabled = enabled;
            Ok(())
        } else {
            Err(BindError::UserInput(format!("no breakpoint #{id}")))
        }
    }

    fn breakpoints(&mut self) -> BindResult<Vec<Breakpoint>> {
        Ok(self.breakpoints.clone())
    }

    fn process_info(&mut self) -> BindResult<ProcessInfo> {
        Ok(ProcessInfo {
            pid: if self.lifecycle.is_alive() {
                Some(Pid::new(4242))
            } else {
                None
            },
            lifecycle: self.lifecycle,
            arch: Architecture::Aarch64,
            endianness: bind_core::Endianness::Little,
            stop_reason: StopReason::Step,
            exit_code: self.exit_code,
        })
    }

    fn threads(&mut self) -> BindResult<Vec<ThreadInfo>> {
        Ok(vec![ThreadInfo {
            id: self.thread,
            name: Some("main".into()),
            is_selected: true,
            run_state: if self.lifecycle.is_stopped() {
                ThreadRunState::Stopped
            } else {
                ThreadRunState::Running
            },
            stop_reason: StopReason::Step,
            pc: Some(addr_of(self.pc_index)),
        }])
    }

    fn frames(&mut self, thread: ThreadId) -> BindResult<Vec<Frame>> {
        self.require_stopped()?;
        let mut frames = Vec::new();
        // Frame 0 is the current instruction.
        frames.push(Frame {
            id: bind_core::FrameId::new(0),
            thread_id: thread,
            index: 0,
            pc: addr_of(self.pc_index),
            sp: Some(Address::new(self.sp)),
            fp: Some(Address::new(self.sp + 16)),
            function: Some(self.program[self.pc_index].symbol.to_string()),
            symbol: Some(self.symbol_for(self.pc_index)),
            source: Some(bind_core::SourceLocation {
                file: "mock.c".into(),
                line: self.program[self.pc_index].line,
                column: None,
            }),
            is_inlined: false,
            is_selected: true,
        });
        // Synthesize caller frames from the return-address stack.
        for (depth, ret_index) in self.call_stack.iter().rev().enumerate() {
            let caller = ret_index.saturating_sub(1);
            frames.push(Frame {
                id: bind_core::FrameId::new((depth + 1) as u64),
                thread_id: thread,
                index: depth + 1,
                pc: addr_of(caller),
                sp: Some(Address::new(self.sp + (depth as u64 + 1) * 16)),
                fp: None,
                function: Some(self.program[caller].symbol.to_string()),
                symbol: Some(self.symbol_for(caller)),
                source: Some(bind_core::SourceLocation {
                    file: "mock.c".into(),
                    line: self.program[caller].line,
                    column: None,
                }),
                is_inlined: false,
                is_selected: false,
            });
        }
        Ok(frames)
    }

    fn registers(&mut self, thread: ThreadId) -> BindResult<RegisterSet> {
        self.require_stopped()?;
        let pc = addr_of(self.pc_index).raw();
        let regs = vec![
            Register::new("x0", self.x0, 64, RegisterRole::General),
            Register::new("x1", 42, 64, RegisterRole::General),
            Register::new("x29", self.sp + 16, 64, RegisterRole::FramePointer),
            Register::new("lr", BASE + 8, 64, RegisterRole::General),
            Register::new("sp", self.sp, 64, RegisterRole::StackPointer),
            Register::new("pc", pc, 64, RegisterRole::ProgramCounter),
            Register::new("cpsr", 0x6000_0000, 32, RegisterRole::Flags),
        ];
        Ok(RegisterSet {
            thread_id: thread,
            registers: regs,
        })
    }

    fn read_memory(&mut self, addr: Address, len: usize) -> BindResult<Vec<u8>> {
        self.require_stopped()?;
        // Return the instruction bytes when reading code, else a deterministic
        // pattern so the memory view has something honest to show.
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let a = addr.raw().wrapping_add(i as u64);
            let byte = match index_of(Address::new(a & !3)) {
                Some(idx) => self.program[idx].bytes[(a & 3) as usize],
                None => (a & 0xff) as u8,
            };
            out.push(byte);
        }
        Ok(out)
    }

    fn disassemble(
        &mut self,
        around: Option<Address>,
        count: usize,
    ) -> BindResult<Vec<Instruction>> {
        let center = around.and_then(index_of).unwrap_or(self.pc_index);
        let start = center.saturating_sub(2);
        let mut out = Vec::new();
        for i in start..(start + count).min(self.program.len()) {
            let insn = &self.program[i];
            out.push(Instruction {
                address: addr_of(i),
                bytes: insn.bytes.to_vec(),
                mnemonic: insn.mnemonic.to_string(),
                operands: insn.operands.to_string(),
                symbol: Some(self.symbol_for(i)),
                source: Some(bind_core::SourceLocation {
                    file: "mock.c".into(),
                    line: insn.line,
                    column: None,
                }),
                branch_target: None,
                is_current: i == self.pc_index,
                has_breakpoint: self.breakpoint_index_at(i).is_some(),
            });
        }
        Ok(out)
    }

    fn modules(&mut self) -> BindResult<Vec<Module>> {
        Ok(vec![Module {
            id: ModuleId::new(1),
            path: "mock".into(),
            load_address: Some(Address::new(BASE)),
            is_jit: false,
            has_debug_info: true,
        }])
    }

    fn memory_regions(&mut self) -> BindResult<Vec<MemoryRegion>> {
        Ok(vec![MemoryRegion {
            start: Address::new(BASE),
            end: Address::new(BASE + self.program.len() as u64 * 4),
            permissions: bind_core::MemoryPermissions {
                read: true,
                write: false,
                execute: true,
            },
            name: Some("mock:.text".into()),
        }])
    }

    fn poll_events(&mut self) -> Vec<DebugEvent> {
        self.events.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bind_core::TargetSpec;

    fn launched() -> MockBackend {
        let mut b = MockBackend::default();
        b.attach(&AttachSpec::Launch(TargetSpec::program("mock")))
            .unwrap();
        let _ = b.poll_events();
        b
    }

    #[test]
    fn launch_stops_at_entry() {
        let mut b = MockBackend::default();
        b.attach(&AttachSpec::Launch(TargetSpec::program("mock")))
            .unwrap();
        let evs = b.poll_events();
        assert!(evs
            .iter()
            .any(|e| matches!(e, DebugEvent::ProcessLaunched { .. })));
        assert!(b.process_info().unwrap().lifecycle.is_stopped());
    }

    #[test]
    fn stepping_changes_registers() {
        let mut b = launched();
        let before = b.registers(ThreadId::new(1)).unwrap();
        // Step onto the call, into helper, ..., until x0 increments.
        for _ in 0..4 {
            b.step(StepKind::Instruction).unwrap();
        }
        let after = b.registers(ThreadId::new(1)).unwrap();
        assert_ne!(before, after, "registers should change while stepping");
    }

    #[test]
    fn breakpoint_by_symbol_is_hit_on_resume() {
        let mut b = launched();
        let bp = b
            .add_breakpoint(&BreakpointLocation::Symbol("helper".into()), None)
            .unwrap();
        assert!(bp.is_resolved());
        let _ = b.poll_events();
        b.resume().unwrap();
        let evs = b.poll_events();
        assert!(evs
            .iter()
            .any(|e| matches!(e, DebugEvent::BreakpointHit { .. })));
        // pc should now be at helper's first instruction.
        let f = b.frames(ThreadId::new(1)).unwrap();
        assert_eq!(f[0].function.as_deref(), Some("helper"));
    }

    #[test]
    fn stepping_out_returns_to_caller() {
        let mut b = launched();
        b.add_breakpoint(&BreakpointLocation::Symbol("helper".into()), None)
            .unwrap();
        b.resume().unwrap();
        let _ = b.poll_events();
        b.step(StepKind::Out).unwrap();
        let f = b.frames(ThreadId::new(1)).unwrap();
        assert_eq!(f[0].function.as_deref(), Some("main"));
    }

    #[test]
    fn queries_fail_when_not_stopped() {
        let mut b = MockBackend::default();
        // Never launched -> Idle.
        assert!(b.registers(ThreadId::new(1)).is_err());
    }
}
