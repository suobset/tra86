use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use tra86_backend::{BackendError, DebugBackend, LaunchRequest};
use tra86_core::{
    Address, Breakpoint, BreakpointId, BreakpointLocation, DisassemblyLine, FrameState,
    MemoryPermissions, MemoryRegion, RegisterBank, RegisterRole, RegisterValue, SourceLocation,
    StopReason, SymbolInfo, ThreadId, ThreadState, ThreadStatus,
};

const CMD_DONE_MARKER: &str = "__TRA86_LDB_CMD_DONE__";

pub struct LldbBackend {
    process: Option<LldbProcess>,
    target_program: Option<String>,
    attached_pid: Option<u32>,
    text_base: Option<Address>,
    breakpoints: Vec<Breakpoint>,
    next_breakpoint_id: BreakpointId,
    last_stop_reason: StopReason,
}

impl LldbBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_process(&mut self) -> Result<&mut LldbProcess, BackendError> {
        if self.process.is_none() {
            self.process = Some(LldbProcess::spawn()?);
        }
        self.process
            .as_mut()
            .ok_or_else(|| BackendError::Internal("LLDB process unavailable".to_string()))
    }

    fn run_command(&mut self, command: &str) -> Result<String, BackendError> {
        let process = self.ensure_process()?;
        process.run_command(command)
    }

    fn run_commands(&mut self, commands: &[String]) -> Result<String, BackendError> {
        let mut out = String::new();
        for cmd in commands {
            let result = self.run_command(cmd)?;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&result);
        }
        Ok(out)
    }

    fn ensure_thread_selected(&mut self, thread_id: ThreadId) -> Result<(), BackendError> {
        let cmd = format!("thread select {thread_id}");
        self.run_command(&cmd).map(|_| ())
    }

    fn update_stop_reason_from_output(&mut self, output: &str) {
        for line in output.lines() {
            if let Some(reason) = parse_stop_reason_line(line) {
                self.last_stop_reason = reason;
            }
            if line.contains("exited with status") {
                let code = line
                    .split("exited with status")
                    .nth(1)
                    .and_then(|tail| tail.trim().split_whitespace().next())
                    .and_then(|token| token.parse::<i32>().ok())
                    .unwrap_or_default();
                self.last_stop_reason = StopReason::Exited(code);
            }
        }
    }

    fn refresh_text_base(&mut self) {
        if let Ok(sections) = self.run_command("image dump sections") {
            self.text_base = parse_text_section_base(&sections);
        }
    }

    fn batch_disassemble(
        &self,
        address: Option<Address>,
        count: usize,
    ) -> Result<Vec<DisassemblyLine>, BackendError> {
        let Some(target) = self.target_program.as_ref() else {
            return Ok(Vec::new());
        };

        let mut cmd = Command::new("lldb");
        cmd.arg("--batch")
            .arg("-o")
            .arg(format!("target create {}", lldb_quote(target)));

        let dis_cmd = match address {
            Some(addr) => format!("disassemble --start-address 0x{addr:x} --count {count}"),
            None => format!("disassemble --name main --count {count}"),
        };
        cmd.arg("-o").arg(dis_cmd);

        let output = cmd.output().map_err(|err| {
            BackendError::Io(format!("failed to run lldb batch disassemble: {err}"))
        })?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(parse_disassembly(&stdout, &self.breakpoints))
    }
}

impl Default for LldbBackend {
    fn default() -> Self {
        Self {
            process: None,
            target_program: None,
            attached_pid: None,
            text_base: None,
            breakpoints: Vec::new(),
            next_breakpoint_id: 1,
            last_stop_reason: StopReason::None,
        }
    }
}

impl DebugBackend for LldbBackend {
    fn backend_name(&self) -> &'static str {
        "lldb"
    }

    fn open_target(&mut self, program: &str) -> Result<(), BackendError> {
        if !Path::new(program).exists() {
            return Err(BackendError::InvalidRequest(format!(
                "program does not exist: {program}"
            )));
        }
        self.target_program = Some(program.to_string());
        self.ensure_process()?;
        let _ = self.run_command("target delete 0");
        let output = self.run_command(&format!("target create {}", lldb_quote(program)))?;
        self.update_stop_reason_from_output(&output);
        self.attached_pid = None;
        self.refresh_text_base();
        Ok(())
    }

    fn launch(&mut self, request: LaunchRequest) -> Result<(), BackendError> {
        if !Path::new(&request.target.program).exists() {
            return Err(BackendError::InvalidRequest(format!(
                "program does not exist: {}",
                request.target.program
            )));
        }

        self.target_program = Some(request.target.program.clone());
        self.ensure_process()?;

        let _ = self.run_command("target delete 0");

        let mut commands = vec![format!(
            "target create {}",
            lldb_quote(&request.target.program)
        )];

        commands.push("breakpoint set --name main".to_string());

        if !request.target.args.is_empty() {
            let args = request
                .target
                .args
                .iter()
                .map(|arg| lldb_quote(arg))
                .collect::<Vec<_>>()
                .join(" ");
            commands.push(format!("settings set target.run-args {args}"));
        }

        commands.push("process launch --stop-at-entry".to_string());

        let output = self.run_commands(&commands)?;
        self.attached_pid = parse_pid(&output);
        self.update_stop_reason_from_output(&output);
        if let Ok(status) = self.run_command("process status") {
            if status.contains("running") {
                let _ = self.run_command("process interrupt");
            }
            self.update_stop_reason_from_output(&status);
        }
        self.refresh_text_base();
        Ok(())
    }

    fn attach(&mut self, pid: u32) -> Result<(), BackendError> {
        let output = self.run_command(&format!("process attach --pid {pid}"))?;
        self.attached_pid = Some(pid);
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn detach(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("process detach")?;
        self.attached_pid = None;
        self.update_stop_reason_from_output(&output);
        self.last_stop_reason = StopReason::Detached;
        Ok(())
    }

    fn kill(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("process kill")?;
        self.update_stop_reason_from_output(&output);
        self.last_stop_reason = StopReason::Exited(0);
        Ok(())
    }

    fn continue_exec(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("process continue")?;
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn step_into(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("thread step-in")?;
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn step_over(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("thread step-over")?;
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn step_out(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("thread step-out")?;
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        let output = self.run_command("process interrupt")?;
        self.update_stop_reason_from_output(&output);
        Ok(())
    }

    fn read_registers(&mut self, thread_id: ThreadId) -> Result<RegisterBank, BackendError> {
        let _ = self.ensure_thread_selected(thread_id);
        let explicit = "register read rip rsp rbp rflags rax rbx rcx rdx rsi rdi r8 r9 r10 r11 r12 r13 r14 r15 pc sp fp x0 x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15 x16 x17 x18 x19 x20 x21 x22 x23 x24 x25 x26 x27 x28 x29 x30 cpsr";
        let output = self
            .run_command(explicit)
            .or_else(|_| self.run_command("register read --all"))?;
        let mut registers = parse_registers(&output);
        if registers.is_empty() {
            registers = parse_registers_loose(&output);
        }
        if registers.is_empty() {
            if let Ok(rows) = self.disassemble(None, 1) {
                if let Some(row) = rows.first() {
                    registers.push(RegisterValue {
                        name: "pc".to_string(),
                        value: format!("0x{:016x}", row.address),
                        size_bits: 64,
                        role: RegisterRole::InstructionPointer,
                    });
                }
            }
        }
        Ok(RegisterBank {
            thread_id,
            bank_name: "general".to_string(),
            registers,
        })
    }

    fn read_memory(&mut self, address: Address, length: usize) -> Result<Vec<u8>, BackendError> {
        let cmd = format!("memory read --size 1 --count {length} 0x{address:x}");
        let output = self.run_command(&cmd)?;
        Ok(parse_memory_bytes(&output))
    }

    fn write_memory(&mut self, address: Address, bytes: &[u8]) -> Result<(), BackendError> {
        if bytes.is_empty() {
            return Ok(());
        }

        let encoded = bytes
            .iter()
            .map(|b| format!("0x{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");

        let cmd = format!("memory write --size 1 0x{address:x} {encoded}");
        self.run_command(&cmd).map(|_| ())
    }

    fn list_threads(&mut self) -> Result<Vec<ThreadState>, BackendError> {
        let output = self.run_command("thread list")?;
        let parsed = parse_threads(&output, self.last_stop_reason.clone());
        if !parsed.is_empty() {
            return Ok(parsed);
        }

        let ip = self
            .disassemble(None, 1)
            .ok()
            .and_then(|rows| rows.first().map(|r| r.address));
        Ok(vec![ThreadState {
            id: 1,
            name: Some("thread-1".to_string()),
            is_current: true,
            status: ThreadStatus::Stopped,
            stop_reason: self.last_stop_reason.clone(),
            instruction_pointer: ip,
        }])
    }

    fn list_frames(&mut self, thread_id: ThreadId) -> Result<Vec<FrameState>, BackendError> {
        self.ensure_thread_selected(thread_id)?;
        let output = self.run_command("thread backtrace")?;
        Ok(parse_frames(&output, thread_id))
    }

    fn set_breakpoint(&mut self, location: BreakpointLocation) -> Result<Breakpoint, BackendError> {
        let cmd = match &location {
            BreakpointLocation::Address(addr) => format!("breakpoint set --address 0x{addr:x}"),
            BreakpointLocation::Symbol(symbol) => format!("breakpoint set --name {symbol}"),
        };

        let output = self.run_command(&cmd)?;
        let parsed_id = parse_breakpoint_id(&output).unwrap_or(self.next_breakpoint_id);
        self.next_breakpoint_id = self.next_breakpoint_id.max(parsed_id + 1);

        let breakpoint = Breakpoint {
            id: parsed_id,
            location,
            enabled: true,
            hit_count: 0,
        };
        self.breakpoints.push(breakpoint.clone());
        Ok(breakpoint)
    }

    fn remove_breakpoint(&mut self, id: BreakpointId) -> Result<(), BackendError> {
        self.run_command(&format!("breakpoint delete {id}"))?;
        self.breakpoints.retain(|bp| bp.id != id);
        Ok(())
    }

    fn disassemble(
        &mut self,
        address: Option<Address>,
        count: usize,
    ) -> Result<Vec<DisassemblyLine>, BackendError> {
        let cmd_primary = match address {
            Some(addr) => format!("disassemble --start-address 0x{addr:x} --count {count}"),
            None => format!("disassemble --pc --count {count}"),
        };

        match self.run_command(&cmd_primary) {
            Ok(output) => {
                let parsed = parse_disassembly(&output, &self.breakpoints);
                if !parsed.is_empty() {
                    return Ok(parsed);
                }
                self.batch_disassemble(address, count)
            }
            Err(_) => {
                if let Some(base) = self.text_base {
                    let by_base = self.run_command(&format!(
                        "disassemble --start-address 0x{base:x} --count {count}"
                    ));
                    if let Ok(output) = by_base {
                        let parsed = parse_disassembly(&output, &self.breakpoints);
                        if !parsed.is_empty() {
                            return Ok(parsed);
                        }
                    }
                }

                if let Ok(fallback) =
                    self.run_command(&format!("disassemble --name main --count {count}"))
                {
                    let parsed = parse_disassembly(&fallback, &self.breakpoints);
                    if !parsed.is_empty() {
                        return Ok(parsed);
                    }
                }

                self.batch_disassemble(address, count)
            }
        }
    }

    fn memory_map(&mut self) -> Result<Vec<MemoryRegion>, BackendError> {
        let output = self.run_command("memory region --all")?;
        let regions = parse_memory_regions(&output);
        if !regions.is_empty() {
            return Ok(regions);
        }

        let fallback = self.run_command("image list")?;
        let target = self.target_program.clone();
        Ok(vec![MemoryRegion {
            start: 0,
            end: 0,
            permissions: MemoryPermissions {
                read: true,
                write: false,
                execute: true,
            },
            pathname: target,
            label: Some(format!("image list output {} bytes", fallback.len())),
        }])
    }

    fn current_instruction(
        &mut self,
        thread_id: ThreadId,
    ) -> Result<Option<Address>, BackendError> {
        self.ensure_thread_selected(thread_id)?;
        let output = self.run_command("register read rip pc eip")?;
        Ok(parse_pc_from_register_output(&output))
    }

    fn current_stop_reason(&mut self) -> Result<StopReason, BackendError> {
        Ok(self.last_stop_reason.clone())
    }

    fn source_location(
        &mut self,
        address: Address,
    ) -> Result<Option<SourceLocation>, BackendError> {
        let output = self.run_command(&format!("image lookup --address 0x{address:x}"))?;
        Ok(parse_source_location(&output))
    }

    fn symbolicate(&mut self, address: Address) -> Result<Option<SymbolInfo>, BackendError> {
        let output = self.run_command(&format!("image lookup --address 0x{address:x}"))?;
        Ok(parse_symbol_info(
            &output,
            address,
            self.target_program.clone(),
        ))
    }

    fn capabilities(&self) -> BTreeMap<String, bool> {
        BTreeMap::from([
            ("launch".to_string(), true),
            ("attach".to_string(), true),
            ("continue".to_string(), true),
            ("step_into".to_string(), true),
            ("step_over".to_string(), true),
            ("step_out".to_string(), true),
            ("pause".to_string(), true),
            ("read_registers".to_string(), true),
            ("read_memory".to_string(), true),
            ("write_memory".to_string(), true),
            ("list_threads".to_string(), true),
            ("list_frames".to_string(), true),
            ("breakpoints".to_string(), true),
            ("disassemble".to_string(), true),
            ("memory_map".to_string(), true),
            ("symbolicate".to_string(), true),
            ("source_location".to_string(), true),
        ])
    }
}

impl Drop for LldbBackend {
    fn drop(&mut self) {
        if let Some(process) = self.process.as_mut() {
            let _ = process.run_command("quit");
            let _ = process.child.kill();
        }
    }
}

struct LldbProcess {
    child: Child,
    stdin: ChildStdin,
    lines_rx: Receiver<String>,
}

impl LldbProcess {
    fn spawn() -> Result<Self, BackendError> {
        let mut child = spawn_lldb_with_pty_fallback()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BackendError::Internal("missing lldb stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BackendError::Internal("missing lldb stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BackendError::Internal("missing lldb stderr".to_string()))?;

        let (tx, rx) = mpsc::channel();
        let tx_stdout = tx.clone();

        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while reader
                .read_line(&mut line)
                .ok()
                .filter(|n| *n > 0)
                .is_some()
            {
                let _ = tx_stdout.send(line.trim_end().to_string());
                line.clear();
            }
        });

        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader
                .read_line(&mut line)
                .ok()
                .filter(|n| *n > 0)
                .is_some()
            {
                let _ = tx.send(line.trim_end().to_string());
                line.clear();
            }
        });

        let mut process = Self {
            child,
            stdin,
            lines_rx: rx,
        };

        process.run_command("settings set stop-disassembly-display never")?;
        process.run_command("settings set interpreter.stop-command-source-on-error false")?;

        Ok(process)
    }

    fn run_command(&mut self, command: &str) -> Result<String, BackendError> {
        while self.lines_rx.try_recv().is_ok() {}

        self.stdin
            .write_all(command.as_bytes())
            .map_err(|err| BackendError::Io(format!("failed to write lldb command: {err}")))?;
        self.stdin
            .write_all(b"\n")
            .map_err(|err| BackendError::Io(format!("failed to write newline: {err}")))?;

        let marker_cmd = format!("script print(\"{CMD_DONE_MARKER}\")\n");
        self.stdin
            .write_all(marker_cmd.as_bytes())
            .map_err(|err| BackendError::Io(format!("failed to write marker command: {err}")))?;

        self.stdin
            .flush()
            .map_err(|err| BackendError::Io(format!("failed to flush lldb stdin: {err}")))?;

        let mut out = String::new();
        loop {
            let line = self
                .lines_rx
                .recv_timeout(Duration::from_secs(20))
                .map_err(|_| BackendError::Timeout)?;
            if line.contains(CMD_DONE_MARKER) {
                break;
            }
            if !line.trim().is_empty() && line.trim() != "(lldb)" {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&line);
            }
        }

        if out.contains("error:") {
            return Err(BackendError::Protocol(out));
        }

        Ok(out)
    }
}

fn spawn_lldb_with_pty_fallback() -> Result<Child, BackendError> {
    #[cfg(target_os = "macos")]
    {
        let via_script = Command::new("script")
            .arg("-q")
            .arg("/dev/null")
            .arg("lldb")
            .arg("--no-lldbinit")
            .arg("-Q")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        if let Ok(child) = via_script {
            return Ok(child);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let via_script = Command::new("script")
            .arg("-q")
            .arg("-c")
            .arg("lldb --no-lldbinit -Q")
            .arg("/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        if let Ok(child) = via_script {
            return Ok(child);
        }
    }

    Command::new("lldb")
        .arg("--no-lldbinit")
        .arg("-Q")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| BackendError::Unavailable(format!("failed to start lldb process: {err}")))
}

fn lldb_quote(input: &str) -> String {
    let escaped = input.replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn parse_pid(output: &str) -> Option<u32> {
    output.lines().find_map(|line| {
        if !line.contains("Process") {
            return None;
        }
        let idx = line.find("Process ")?;
        let token = line[idx + 8..].split_whitespace().next()?;
        token.parse::<u32>().ok()
    })
}

fn parse_breakpoint_id(output: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        if !line.contains("Breakpoint") {
            return None;
        }
        let idx = line.find("Breakpoint ")?;
        let token = line[idx + 11..]
            .split(':')
            .next()
            .and_then(|text| text.split_whitespace().next())?;
        token.parse::<u64>().ok()
    })
}

fn parse_stop_reason_line(line: &str) -> Option<StopReason> {
    if line.contains("stop reason = breakpoint") {
        return Some(StopReason::Breakpoint(0));
    }
    if line.contains("stop reason = signal") {
        let signal = line
            .split("stop reason =")
            .nth(1)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "signal".to_string());
        return Some(StopReason::Signal(signal));
    }
    if line.contains("stop reason = step") {
        return Some(StopReason::Step);
    }
    None
}

fn parse_registers(output: &str) -> Vec<RegisterValue> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Some((raw_name, raw_value)) = line.split_once('=') else {
            continue;
        };

        let name = raw_name.trim().to_string();
        if name.is_empty() || name.contains(' ') {
            continue;
        }

        let value = raw_value
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        if value.is_empty() {
            continue;
        }

        out.push(RegisterValue {
            role: register_role(&name),
            name,
            value,
            size_bits: 64,
        });
    }
    out
}

fn parse_registers_loose(output: &str) -> Vec<RegisterValue> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };

        let name = left
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();

        if name.is_empty() {
            continue;
        }

        let value = right
            .split_whitespace()
            .find(|token| token.starts_with("0x"))
            .unwrap_or("")
            .to_string();
        if value.is_empty() {
            continue;
        }

        out.push(RegisterValue {
            role: register_role(&name),
            name,
            value,
            size_bits: 64,
        });
    }
    out
}

fn register_role(name: &str) -> RegisterRole {
    match name {
        "rip" | "pc" | "eip" => RegisterRole::InstructionPointer,
        "rsp" | "sp" => RegisterRole::StackPointer,
        "rbp" | "fp" => RegisterRole::FramePointer,
        "rflags" | "eflags" | "cpsr" => RegisterRole::Flags,
        _ => RegisterRole::General,
    }
}

fn parse_threads(output: &str, default_reason: StopReason) -> Vec<ThreadState> {
    let mut out = Vec::new();
    for line in output.lines() {
        if !line.contains("thread #") {
            continue;
        }

        let is_current = line.trim_start().starts_with('*');
        let id = line
            .split("thread #")
            .nth(1)
            .map(|tail| {
                tail.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
            .and_then(|token| token.parse::<u64>().ok())
            .unwrap_or(0);

        let ip = line
            .split_whitespace()
            .find(|token| token.starts_with("0x"))
            .and_then(|token| u64::from_str_radix(token.trim_start_matches("0x"), 16).ok());

        let reason = line
            .split("stop reason =")
            .nth(1)
            .map(|tail| StopReason::Unknown(tail.trim().to_string()))
            .unwrap_or_else(|| default_reason.clone());

        out.push(ThreadState {
            id,
            name: None,
            is_current,
            status: ThreadStatus::Stopped,
            stop_reason: reason,
            instruction_pointer: ip,
        });
    }

    out
}

fn parse_frames(output: &str, thread_id: ThreadId) -> Vec<FrameState> {
    let mut frames = Vec::new();

    for line in output.lines() {
        if !line.contains("frame #") {
            continue;
        }

        let index = line
            .split("frame #")
            .nth(1)
            .and_then(|tail| tail.split(':').next())
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);

        let ip = extract_address_token(line).unwrap_or(0);

        let function = line.split('`').nth(1).map(|tail| {
            tail.split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_string()
        });

        frames.push(FrameState {
            id: index as u64,
            thread_id,
            index,
            instruction_pointer: ip,
            stack_pointer: None,
            frame_pointer: None,
            function,
            symbol: None,
            source: parse_source_location(line),
        });
    }

    frames
}

fn parse_disassembly(output: &str, breakpoints: &[Breakpoint]) -> Vec<DisassemblyLine> {
    let mut out = Vec::new();
    let mut current_function: Option<String> = None;

    for line in output.lines() {
        if line.contains('`') && line.trim_end().ends_with(':') && !line.contains("0x") {
            current_function = line
                .split('`')
                .nth(1)
                .map(|v| v.trim_end_matches(':').trim().to_string());
            continue;
        }

        if !line.contains("0x") || !line.contains(':') {
            continue;
        }

        let is_current = line.contains("->");
        let Some(address) = extract_address_token(line) else {
            continue;
        };

        let instruction = line.split(':').nth(1).map(str::trim).unwrap_or("");
        let mut parts = instruction.split_whitespace();
        let mnemonic = parts.next().unwrap_or("nop").to_string();
        let operands = parts.collect::<Vec<_>>().join(" ");
        let branch_target = operands
            .split(|c: char| c == ',' || c.is_whitespace())
            .find(|token| token.starts_with("0x"))
            .and_then(|token| u64::from_str_radix(token.trim_start_matches("0x"), 16).ok());

        let has_breakpoint = breakpoints.iter().any(|bp| {
            bp.enabled
                && matches!(bp.location, BreakpointLocation::Address(addr) if addr == address)
        });

        out.push(DisassemblyLine {
            address,
            bytes: Vec::new(),
            mnemonic,
            operands,
            function: current_function.clone(),
            source: None,
            branch_target,
            is_current,
            has_breakpoint,
        });
    }

    out
}

fn extract_address_token(line: &str) -> Option<Address> {
    for token in line.split_whitespace() {
        if token.starts_with("0x") {
            if let Ok(addr) = u64::from_str_radix(token.trim_start_matches("0x"), 16) {
                return Some(addr);
            }
        }

        if let Some(start) = token.find("0x") {
            let hex = token[start + 2..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect::<String>();
            if !hex.is_empty() {
                if let Ok(addr) = u64::from_str_radix(&hex, 16) {
                    return Some(addr);
                }
            }
        }
    }
    None
}

fn parse_memory_bytes(output: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in output.lines() {
        for token in line.split_whitespace() {
            if token.starts_with("0x") && token.len() == 4 {
                if let Ok(byte) = u8::from_str_radix(token.trim_start_matches("0x"), 16) {
                    out.push(byte);
                }
            }
        }
    }
    out
}

fn parse_memory_regions(output: &str) -> Vec<MemoryRegion> {
    let mut regions = Vec::new();

    for line in output.lines() {
        if !(line.contains('[') && line.contains(')') && line.contains('-')) {
            continue;
        }

        let start = line
            .split('[')
            .nth(1)
            .and_then(|tail| tail.split('-').next())
            .map(str::trim)
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());

        let end = line
            .split('-')
            .nth(1)
            .and_then(|tail| tail.split(')').next())
            .map(str::trim)
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());

        let perms_token = line
            .split_whitespace()
            .find(|token| {
                token.len() == 3 && token.chars().all(|c| matches!(c, 'r' | 'w' | 'x' | '-'))
            })
            .unwrap_or("---");

        if let (Some(start), Some(end)) = (start, end) {
            regions.push(MemoryRegion {
                start,
                end,
                permissions: MemoryPermissions {
                    read: perms_token.as_bytes()[0] == b'r',
                    write: perms_token.as_bytes()[1] == b'w',
                    execute: perms_token.as_bytes()[2] == b'x',
                },
                pathname: line.split_whitespace().last().map(|v| v.to_string()),
                label: None,
            });
        }
    }

    regions
}

fn parse_text_section_base(output: &str) -> Option<Address> {
    for line in output.lines() {
        if !(line.contains("__TEXT") || line.contains(".text")) {
            continue;
        }

        if let Some(addr) = extract_address_token(line) {
            return Some(addr);
        }
    }
    None
}

fn parse_pc_from_register_output(output: &str) -> Option<u64> {
    parse_registers(output)
        .into_iter()
        .find(|reg| matches!(reg.role, RegisterRole::InstructionPointer))
        .and_then(|reg| parse_addr(&reg.value))
}

fn parse_source_location(line_or_output: &str) -> Option<SourceLocation> {
    for line in line_or_output.lines() {
        if let Some(rest) = line.split(" at ").nth(1) {
            let mut pieces = rest.rsplitn(3, ':').collect::<Vec<_>>();
            pieces.reverse();
            if pieces.len() >= 2 {
                let file = pieces[0].to_string();
                let line_num = pieces[1].parse::<u32>().ok()?;
                let column = pieces.get(2).and_then(|v| v.parse::<u32>().ok());
                return Some(SourceLocation {
                    file,
                    line: line_num,
                    column,
                });
            }
        }
    }
    None
}

fn parse_symbol_info(output: &str, address: Address, module: Option<String>) -> Option<SymbolInfo> {
    let mut name: Option<String> = None;

    for line in output.lines() {
        if line.contains("Summary:") {
            name = line
                .split("Summary:")
                .nth(1)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
        }
        if line.contains("name =") {
            name = line
                .split("name =")
                .nth(1)
                .map(|v| v.trim().trim_matches('"').to_string())
                .filter(|v| !v.is_empty());
        }
    }

    name.map(|n| SymbolInfo {
        name: n,
        module,
        offset: address,
    })
}

fn parse_addr(value: &str) -> Option<u64> {
    let cleaned = value.trim().trim_start_matches("0x");
    u64::from_str_radix(cleaned, 16).ok()
}
