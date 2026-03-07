use std::collections::BTreeMap;

use tra86_core::{
    Address, Breakpoint, BreakpointId, BreakpointLocation, DisassemblyLine, FrameState,
    MemoryPermissions, MemoryRegion, RegisterBank, RegisterRole, RegisterValue, SourceLocation,
    StopReason, SymbolInfo, TargetBinary, ThreadId, ThreadState, ThreadStatus,
};

use crate::{BackendError, DebugBackend, LaunchRequest};

const BASE_ADDR: Address = 0x1000;
const STACK_BASE: Address = 0x7fff_ff00;

pub struct MockBackend {
    target: Option<TargetBinary>,
    attached_pid: Option<u32>,
    running: bool,
    thread_id: ThreadId,
    pc: Address,
    sp: Address,
    bp: Address,
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    flags: u64,
    next_breakpoint_id: BreakpointId,
    breakpoints: Vec<Breakpoint>,
    stop_reason: StopReason,
    program: Vec<DisassemblyLine>,
    memory: Vec<u8>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            target: None,
            attached_pid: None,
            running: false,
            thread_id: 1,
            pc: BASE_ADDR,
            sp: STACK_BASE,
            bp: STACK_BASE,
            rax: 1,
            rbx: 2,
            rcx: 3,
            rdx: 4,
            flags: 0x202,
            next_breakpoint_id: 1,
            breakpoints: Vec::new(),
            stop_reason: StopReason::None,
            program: sample_program(),
            memory: sample_memory(),
        }
    }
}

impl MockBackend {
    fn ensure_running(&self) -> Result<(), BackendError> {
        if self.running {
            Ok(())
        } else {
            Err(BackendError::ProcessNotRunning)
        }
    }

    fn execute_one(&mut self) {
        let current = self.pc;
        self.rax = self.rax.wrapping_add(1);
        self.rbx ^= self.rax;
        self.rcx = self.rcx.wrapping_add(4);
        self.sp = self.sp.wrapping_sub(8);
        self.flags ^= 0x40;

        let next = if current >= BASE_ADDR + ((self.program.len() - 1) as u64 * 4) {
            BASE_ADDR
        } else {
            current + 4
        };

        self.pc = next;
        self.stop_reason = StopReason::Step;

        if let Some(bp) = self.breakpoints.iter_mut().find(|bp| {
            bp.enabled
                && matches!(bp.location, BreakpointLocation::Address(addr) if addr == self.pc)
        }) {
            bp.hit_count += 1;
            self.stop_reason = StopReason::Breakpoint(bp.id);
        }

        let stack_idx = (self.sp as usize).wrapping_sub(STACK_BASE as usize) % self.memory.len();
        self.memory[stack_idx] = self.rax as u8;
    }
}

impl DebugBackend for MockBackend {
    fn backend_name(&self) -> &'static str {
        "mock"
    }

    fn open_target(&mut self, program: &str) -> Result<(), BackendError> {
        self.target = Some(TargetBinary {
            program: program.to_string(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        });
        self.running = false;
        self.stop_reason = StopReason::None;
        self.pc = BASE_ADDR;
        Ok(())
    }

    fn launch(&mut self, request: LaunchRequest) -> Result<(), BackendError> {
        self.target = Some(request.target);
        self.running = true;
        self.attached_pid = Some(42000);
        self.pc = BASE_ADDR;
        self.sp = STACK_BASE;
        self.bp = STACK_BASE;
        self.stop_reason = StopReason::Step;
        Ok(())
    }

    fn attach(&mut self, pid: u32) -> Result<(), BackendError> {
        self.attached_pid = Some(pid);
        self.running = true;
        self.stop_reason = StopReason::Step;
        Ok(())
    }

    fn detach(&mut self) -> Result<(), BackendError> {
        self.running = false;
        self.attached_pid = None;
        self.stop_reason = StopReason::Detached;
        Ok(())
    }

    fn kill(&mut self) -> Result<(), BackendError> {
        self.running = false;
        self.stop_reason = StopReason::Exited(0);
        Ok(())
    }

    fn continue_exec(&mut self) -> Result<(), BackendError> {
        self.ensure_running()?;
        for _ in 0..8 {
            self.execute_one();
            if matches!(self.stop_reason, StopReason::Breakpoint(_)) {
                break;
            }
        }
        Ok(())
    }

    fn step_into(&mut self) -> Result<(), BackendError> {
        self.ensure_running()?;
        self.execute_one();
        Ok(())
    }

    fn step_over(&mut self) -> Result<(), BackendError> {
        self.step_into()
    }

    fn step_out(&mut self) -> Result<(), BackendError> {
        self.ensure_running()?;
        self.pc = BASE_ADDR + 0x24;
        self.stop_reason = StopReason::Step;
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.ensure_running()?;
        self.stop_reason = StopReason::Signal("SIGSTOP".to_string());
        Ok(())
    }

    fn read_registers(&mut self, thread_id: ThreadId) -> Result<RegisterBank, BackendError> {
        if thread_id != self.thread_id {
            return Err(BackendError::InvalidRequest(format!(
                "unknown thread {thread_id}"
            )));
        }

        Ok(RegisterBank {
            thread_id,
            bank_name: "general".to_string(),
            registers: vec![
                RegisterValue {
                    name: "rip".to_string(),
                    value: format!("0x{:016x}", self.pc),
                    size_bits: 64,
                    role: RegisterRole::InstructionPointer,
                },
                RegisterValue {
                    name: "rsp".to_string(),
                    value: format!("0x{:016x}", self.sp),
                    size_bits: 64,
                    role: RegisterRole::StackPointer,
                },
                RegisterValue {
                    name: "rbp".to_string(),
                    value: format!("0x{:016x}", self.bp),
                    size_bits: 64,
                    role: RegisterRole::FramePointer,
                },
                RegisterValue {
                    name: "rax".to_string(),
                    value: format!("0x{:016x}", self.rax),
                    size_bits: 64,
                    role: RegisterRole::General,
                },
                RegisterValue {
                    name: "rbx".to_string(),
                    value: format!("0x{:016x}", self.rbx),
                    size_bits: 64,
                    role: RegisterRole::General,
                },
                RegisterValue {
                    name: "rcx".to_string(),
                    value: format!("0x{:016x}", self.rcx),
                    size_bits: 64,
                    role: RegisterRole::General,
                },
                RegisterValue {
                    name: "rdx".to_string(),
                    value: format!("0x{:016x}", self.rdx),
                    size_bits: 64,
                    role: RegisterRole::General,
                },
                RegisterValue {
                    name: "rflags".to_string(),
                    value: format!("0x{:016x}", self.flags),
                    size_bits: 64,
                    role: RegisterRole::Flags,
                },
            ],
        })
    }

    fn read_memory(&mut self, address: Address, length: usize) -> Result<Vec<u8>, BackendError> {
        if self.target.is_none() {
            return Err(BackendError::InvalidRequest(
                "no target loaded for memory view".to_string(),
            ));
        }

        if length == 0 {
            return Ok(Vec::new());
        }

        let start = (address as usize) % self.memory.len();
        let mut out = Vec::with_capacity(length);
        for i in 0..length {
            out.push(self.memory[(start + i) % self.memory.len()]);
        }
        Ok(out)
    }

    fn write_memory(&mut self, address: Address, bytes: &[u8]) -> Result<(), BackendError> {
        if self.target.is_none() {
            return Err(BackendError::InvalidRequest(
                "no target loaded for memory write".to_string(),
            ));
        }
        let memory_len = self.memory.len();
        let start = (address as usize) % memory_len;
        for (idx, value) in bytes.iter().enumerate() {
            self.memory[(start + idx) % memory_len] = *value;
        }
        Ok(())
    }

    fn list_threads(&mut self) -> Result<Vec<ThreadState>, BackendError> {
        self.ensure_running()?;
        Ok(vec![ThreadState {
            id: self.thread_id,
            name: Some("main".to_string()),
            is_current: true,
            status: ThreadStatus::Stopped,
            stop_reason: self.stop_reason.clone(),
            instruction_pointer: Some(self.pc),
        }])
    }

    fn list_frames(&mut self, thread_id: ThreadId) -> Result<Vec<FrameState>, BackendError> {
        if thread_id != self.thread_id {
            return Err(BackendError::InvalidRequest(format!(
                "unknown thread {thread_id}"
            )));
        }

        Ok(vec![FrameState {
            id: 0,
            thread_id,
            index: 0,
            instruction_pointer: self.pc,
            stack_pointer: Some(self.sp),
            frame_pointer: Some(self.bp),
            function: Some("main".to_string()),
            symbol: Some(SymbolInfo {
                name: "main".to_string(),
                module: self.target.as_ref().map(|t| t.program.clone()),
                offset: self.pc.saturating_sub(BASE_ADDR),
            }),
            source: Some(SourceLocation {
                file: "mock/main.c".to_string(),
                line: 42,
                column: Some(1),
            }),
        }])
    }

    fn set_breakpoint(&mut self, location: BreakpointLocation) -> Result<Breakpoint, BackendError> {
        let bp = Breakpoint {
            id: self.next_breakpoint_id,
            location,
            enabled: true,
            hit_count: 0,
        };
        self.next_breakpoint_id += 1;
        self.breakpoints.push(bp.clone());
        Ok(bp)
    }

    fn remove_breakpoint(&mut self, id: BreakpointId) -> Result<(), BackendError> {
        let len_before = self.breakpoints.len();
        self.breakpoints.retain(|bp| bp.id != id);
        if len_before == self.breakpoints.len() {
            return Err(BackendError::InvalidRequest(format!(
                "breakpoint {id} not found"
            )));
        }
        Ok(())
    }

    fn disassemble(
        &mut self,
        address: Option<Address>,
        count: usize,
    ) -> Result<Vec<DisassemblyLine>, BackendError> {
        if self.target.is_none() {
            return Err(BackendError::InvalidRequest(
                "no target loaded for disassembly".to_string(),
            ));
        }

        let center = address.unwrap_or(self.pc);
        let mut lines = Vec::with_capacity(count);
        let mut cursor = center.saturating_sub(((count / 2) as u64) * 4);

        for _ in 0..count {
            let mut line = self
                .program
                .iter()
                .find(|candidate| candidate.address == cursor)
                .cloned()
                .unwrap_or_else(|| DisassemblyLine {
                    address: cursor,
                    bytes: vec![0x90],
                    mnemonic: "nop".to_string(),
                    operands: String::new(),
                    function: Some("unknown".to_string()),
                    source: None,
                    branch_target: None,
                    is_current: false,
                    has_breakpoint: false,
                });

            line.is_current = cursor == self.pc;
            line.has_breakpoint = self.breakpoints.iter().any(|bp| {
                bp.enabled
                    && matches!(bp.location, BreakpointLocation::Address(addr) if addr == cursor)
            });

            lines.push(line);
            cursor = cursor.saturating_add(4);
        }

        Ok(lines)
    }

    fn memory_map(&mut self) -> Result<Vec<MemoryRegion>, BackendError> {
        self.ensure_running()?;
        Ok(vec![
            MemoryRegion {
                start: BASE_ADDR,
                end: BASE_ADDR + 0x4000,
                permissions: MemoryPermissions {
                    read: true,
                    write: false,
                    execute: true,
                },
                pathname: self.target.as_ref().map(|t| t.program.clone()),
                label: Some(".text".to_string()),
            },
            MemoryRegion {
                start: 0x2000_0000,
                end: 0x2000_2000,
                permissions: MemoryPermissions {
                    read: true,
                    write: true,
                    execute: false,
                },
                pathname: None,
                label: Some("heap".to_string()),
            },
            MemoryRegion {
                start: STACK_BASE - 0x8000,
                end: STACK_BASE + 0x1000,
                permissions: MemoryPermissions {
                    read: true,
                    write: true,
                    execute: false,
                },
                pathname: None,
                label: Some("stack".to_string()),
            },
        ])
    }

    fn current_instruction(
        &mut self,
        thread_id: ThreadId,
    ) -> Result<Option<Address>, BackendError> {
        if thread_id != self.thread_id {
            return Ok(None);
        }
        Ok(Some(self.pc))
    }

    fn current_stop_reason(&mut self) -> Result<StopReason, BackendError> {
        Ok(self.stop_reason.clone())
    }

    fn source_location(
        &mut self,
        address: Address,
    ) -> Result<Option<SourceLocation>, BackendError> {
        if (BASE_ADDR..=(BASE_ADDR + 0x28)).contains(&address) {
            Ok(Some(SourceLocation {
                file: "mock/main.c".to_string(),
                line: 40 + ((address - BASE_ADDR) / 4) as u32,
                column: Some(1),
            }))
        } else {
            Ok(None)
        }
    }

    fn symbolicate(&mut self, address: Address) -> Result<Option<SymbolInfo>, BackendError> {
        if (BASE_ADDR..=(BASE_ADDR + 0x2c)).contains(&address) {
            Ok(Some(SymbolInfo {
                name: "main".to_string(),
                module: self.target.as_ref().map(|t| t.program.clone()),
                offset: address.saturating_sub(BASE_ADDR),
            }))
        } else {
            Ok(None)
        }
    }

    fn capabilities(&self) -> BTreeMap<String, bool> {
        BTreeMap::from([
            ("launch".to_string(), true),
            ("attach".to_string(), true),
            ("read_memory".to_string(), true),
            ("write_memory".to_string(), true),
            ("source_location".to_string(), true),
            ("symbolicate".to_string(), true),
        ])
    }
}

fn sample_program() -> Vec<DisassemblyLine> {
    let rows = [
        (0x1000, [0x55, 0x48, 0x89, 0xe5], "push", "rbp"),
        (0x1004, [0x48, 0x83, 0xec, 0x20], "sub", "rsp, 0x20"),
        (0x1008, [0x48, 0x8b, 0x45, 0xf8], "mov", "rax, [rbp-0x8]"),
        (0x100c, [0x48, 0x83, 0xc0, 0x01], "add", "rax, 0x1"),
        (0x1010, [0x48, 0x89, 0x45, 0xf8], "mov", "[rbp-0x8], rax"),
        (0x1014, [0xe8, 0x07, 0x00, 0x00], "call", "0x1020"),
        (0x1018, [0x48, 0x83, 0xc4, 0x20], "add", "rsp, 0x20"),
        (0x101c, [0x5d, 0xc3, 0x90, 0x90], "ret", ""),
        (0x1020, [0x48, 0x31, 0xc0, 0x90], "xor", "rax, rax"),
        (0x1024, [0x0f, 0x05, 0x90, 0x90], "syscall", ""),
        (0x1028, [0xc3, 0x90, 0x90, 0x90], "ret", ""),
    ];

    rows.into_iter()
        .map(|(address, bytes, mnemonic, operands)| DisassemblyLine {
            address,
            bytes: bytes.to_vec(),
            mnemonic: mnemonic.to_string(),
            operands: operands.to_string(),
            function: Some("main".to_string()),
            source: Some(SourceLocation {
                file: "mock/main.c".to_string(),
                line: 40 + ((address - BASE_ADDR) / 4) as u32,
                column: Some(1),
            }),
            branch_target: (mnemonic == "call").then_some(0x1020),
            is_current: address == BASE_ADDR,
            has_breakpoint: false,
        })
        .collect()
}

fn sample_memory() -> Vec<u8> {
    (0..0x10000).map(|v| (v % 255) as u8).collect()
}
