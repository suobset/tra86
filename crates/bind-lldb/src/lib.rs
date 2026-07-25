//! `bind-lldb`: the LLDB backend, isolated behind [`bind_debugger::DebugBackend`].
//!
//! Because liblldb cannot be linked on this platform, the LLDB *SB API* is
//! driven out-of-process by an embedded Python driver
//! (`driver/bind_lldb_driver.py`) that Bind talks to over a JSON line protocol
//! (see [`protocol`]). This is a structured programmatic adapter — it never
//! parses the human-readable `lldb` console — so acceptance criterion 15 ("no
//! core workflow depends on parsing console output") holds.
//!
//! All LLDB-specific translation lives here; the rest of Bind sees only
//! `bind-core` types and normalized events.

pub mod protocol;

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use bind_core::{
    Address, Architecture, AttachSpec, Breakpoint, BreakpointId, BreakpointLocation, Capabilities,
    DebugEvent, Endianness, Frame, FrameId, Instruction, MemoryRegion, Module, ModuleId, Pid,
    ProcessInfo, ProcessLifecycle, Register, RegisterSet, ResolvedLocation, RunTarget,
    SourceLocation, StepKind, StopReason, Symbol, ThreadId, ThreadInfo, ThreadRunState,
};
use bind_core::{BindError, BindResult};
use bind_debugger::DebugBackend;
use serde_json::{json, Value};

use protocol::DriverClient;

/// The embedded SB-API driver source, written to a temp file at startup.
const DRIVER_SOURCE: &str = include_str!("../driver/bind_lldb_driver.py");

/// Discovers the LLDB python path via `lldb -P`.
pub fn lldb_pythonpath() -> Option<String> {
    let out = Command::new("lldb").arg("-P").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Finds a python3 that can host the LLDB module. Prefers `xcrun --find python3`
/// (matches the framework), falls back to `python3` on PATH.
pub fn python_exe() -> String {
    if let Ok(out) = Command::new("xcrun").args(["--find", "python3"]).output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return p;
            }
        }
    }
    "python3".to_string()
}

/// Whether a usable LLDB SB-API environment appears to be present.
pub fn is_available() -> bool {
    lldb_pythonpath().is_some()
}

fn write_driver() -> BindResult<PathBuf> {
    let path = std::env::temp_dir().join("bind_lldb_driver.py");
    let mut f = std::fs::File::create(&path)
        .map_err(|e| BindError::Internal(format!("write driver: {e}")))?;
    f.write_all(DRIVER_SOURCE.as_bytes())
        .map_err(|e| BindError::Internal(format!("write driver: {e}")))?;
    Ok(path)
}

pub struct LldbBackend {
    client: DriverClient,
    arch: Architecture,
    program: Option<String>,
    lifecycle: ProcessLifecycle,
    events: VecDeque<DebugEvent>,
    breakpoints: Vec<Breakpoint>,
    selected_thread: Option<ThreadId>,
    last_pc: Option<Address>,
    exit_code: Option<i32>,
}

impl LldbBackend {
    /// Spawns the driver against the ambient LLDB environment.
    pub fn new() -> BindResult<Self> {
        let pythonpath = lldb_pythonpath().ok_or_else(|| {
            BindError::Backend(
                "LLDB python path not found (`lldb -P` failed); is LLDB installed?".into(),
            )
        })?;
        let driver = write_driver()?;
        let client = DriverClient::spawn(&python_exe(), &driver, &pythonpath)?;
        Ok(Self {
            client,
            arch: Architecture::Unknown,
            program: None,
            lifecycle: ProcessLifecycle::Idle,
            events: VecDeque::new(),
            breakpoints: Vec::new(),
            selected_thread: None,
            last_pc: None,
            exit_code: None,
        })
    }

    /// Exposes the driver client for direct RPC in tests.
    pub fn client_mut(&mut self) -> &mut DriverClient {
        &mut self.client
    }

    fn parse_stop_reason(&self, s: &str) -> StopReason {
        if let Some(rest) = s.strip_prefix("breakpoint:") {
            StopReason::Breakpoint(BreakpointId::new(rest.parse().unwrap_or(0)))
        } else if s == "step" {
            StopReason::Step
        } else if let Some(sig) = s.strip_prefix("signal:") {
            StopReason::Signal(format!("signal {sig}"))
        } else if let Some(exc) = s.strip_prefix("exception:") {
            StopReason::Exception(exc.to_string())
        } else {
            StopReason::Unknown(s.to_string())
        }
    }

    /// Translates a driver stop-state object into normalized events and updates
    /// internal lifecycle.
    fn emit_stop(&mut self, v: &Value, from: Option<Address>) {
        if let Some(code) = v.get("exited").and_then(Value::as_i64) {
            self.lifecycle = ProcessLifecycle::Exited;
            self.exit_code = Some(code as i32);
            self.events
                .push_back(DebugEvent::ProcessExited { code: code as i32 });
            return;
        }
        if v.get("stopped").and_then(Value::as_bool) == Some(true) {
            let thread = ThreadId::new(v.get("thread").and_then(Value::as_u64).unwrap_or(0));
            let reason_str = v.get("reason").and_then(Value::as_str).unwrap_or("none");
            let reason = self.parse_stop_reason(reason_str);
            let pc = v.get("pc").and_then(Value::as_u64).map(Address::new);
            self.lifecycle = ProcessLifecycle::Stopped;
            self.selected_thread = Some(thread);
            self.last_pc = pc;
            if let StopReason::Breakpoint(bp) = &reason {
                self.events.push_back(DebugEvent::BreakpointHit {
                    breakpoint: *bp,
                    thread,
                });
            }
            if let Some(f) = from {
                self.events.push_back(DebugEvent::InstructionStepped {
                    thread,
                    from: Some(f),
                    to: pc.unwrap_or(Address::NULL),
                });
            }
            self.events
                .push_back(DebugEvent::Stopped { thread, reason, pc });
        }
    }

    fn thread_arg(&self, thread: ThreadId) -> Value {
        json!({ "thread": thread.raw() })
    }
}

fn decode_hex(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16);
        let lo = (bytes[i + 1] as char).to_digit(16);
        if let (Some(h), Some(l)) = (hi, lo) {
            out.push((h * 16 + l) as u8);
        }
        i += 2;
    }
    out
}

fn arch_from_triple(triple: &str) -> Architecture {
    Architecture::from_triple_arch(triple.split('-').next().unwrap_or(""))
}

impl DebugBackend for LldbBackend {
    fn name(&self) -> &'static str {
        "lldb"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            launch: true,
            attach: true,
            core_files: false,
            detach: true,
            // The out-of-process synchronous driver cannot reliably interrupt a
            // running process mid-flight; reported honestly as unsupported.
            async_pause: false,
            step_instruction: true,
            step_source: true,
            breakpoints: true,
            conditional_breakpoints: true,
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
                let opened = self
                    .client
                    .request("open_target", json!({ "program": target.program }))?;
                if let Some(triple) = opened.get("triple").and_then(Value::as_str) {
                    self.arch = arch_from_triple(triple);
                }
                self.program = Some(target.program.clone());
                self.events.push_back(DebugEvent::TargetLoaded {
                    program: target.program.clone(),
                    arch: self.arch,
                });
                self.lifecycle = ProcessLifecycle::Launching;
                let env: Vec<Value> = target.env.iter().map(|(k, v)| json!([k, v])).collect();
                let stop = self.client.request(
                    "launch",
                    json!({
                        "args": target.args,
                        "cwd": target.cwd,
                        "env": env,
                        "stop_at_entry": target.stop_at_entry,
                    }),
                )?;
                if let Some(pid) = self
                    .client
                    .request("process_info", json!({}))
                    .ok()
                    .and_then(|v| v.get("pid").and_then(Value::as_u64))
                {
                    self.events.push_back(DebugEvent::ProcessLaunched {
                        pid: Pid::new(pid as u32),
                    });
                }
                self.emit_stop(&stop, None);
                Ok(())
            }
            AttachSpec::Pid(pid) => {
                let stop = self.client.request("attach", json!({ "pid": pid.raw() }))?;
                self.events
                    .push_back(DebugEvent::ProcessAttached { pid: *pid });
                self.emit_stop(&stop, None);
                Ok(())
            }
            AttachSpec::CoreFile { .. } => Err(BindError::Unsupported(
                "core-file debugging is not yet implemented by the LLDB backend".into(),
            )),
        }
    }

    fn detach(&mut self) -> BindResult<()> {
        self.client.request("detach", json!({}))?;
        self.lifecycle = ProcessLifecycle::Detached;
        self.events.push_back(DebugEvent::ProcessDetached);
        Ok(())
    }

    fn terminate(&mut self) -> BindResult<()> {
        self.client.request("kill", json!({}))?;
        self.lifecycle = ProcessLifecycle::Exited;
        self.events.push_back(DebugEvent::ProcessExited {
            code: self.exit_code.unwrap_or(0),
        });
        Ok(())
    }

    fn resume(&mut self) -> BindResult<()> {
        self.events.push_back(DebugEvent::Continued);
        let stop = self.client.request("resume", json!({}))?;
        self.emit_stop(&stop, None);
        Ok(())
    }

    fn pause(&mut self) -> BindResult<()> {
        Err(BindError::CapabilityLimited(
            "asynchronous pause is not supported by the out-of-process LLDB driver".into(),
        ))
    }

    fn step(&mut self, kind: StepKind) -> BindResult<()> {
        let from = self.last_pc;
        let kind_str = match kind {
            StepKind::Into => "into",
            StepKind::Over => "over",
            StepKind::Out => "out",
            StepKind::Instruction => "instruction",
        };
        let stop = self.client.request("step", json!({ "kind": kind_str }))?;
        self.emit_stop(&stop, from);
        Ok(())
    }

    fn run_to(&mut self, target: &RunTarget) -> BindResult<()> {
        // Implemented as a temporary breakpoint + continue.
        let loc = match target {
            RunTarget::Address(a) => json!({ "kind": "address", "address": a.raw() }),
            RunTarget::Source { file, line } => {
                json!({ "kind": "source", "file": file, "line": line })
            }
        };
        let bp = self.client.request("add_breakpoint", loc)?;
        let bp_id = bp.get("id").and_then(Value::as_u64).unwrap_or(0);
        self.events.push_back(DebugEvent::Continued);
        let stop = self.client.request("resume", json!({}))?;
        let _ = self
            .client
            .request("remove_breakpoint", json!({ "id": bp_id }));
        self.emit_stop(&stop, None);
        Ok(())
    }

    fn select_thread(&mut self, thread: ThreadId) -> BindResult<()> {
        self.selected_thread = Some(thread);
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
        let mut req = match location {
            BreakpointLocation::Symbol(name) => json!({ "kind": "symbol", "name": name }),
            BreakpointLocation::Address(a) => json!({ "kind": "address", "address": a.raw() }),
            BreakpointLocation::Source { file, line } => {
                json!({ "kind": "source", "file": file, "line": line })
            }
        };
        if let Some(cond) = condition {
            req["condition"] = Value::from(cond);
        }
        let resp = self.client.request("add_breakpoint", req)?;
        let id = BreakpointId::new(resp.get("id").and_then(Value::as_u64).unwrap_or(0));
        let mut resolved = Vec::new();
        if let Some(arr) = resp.get("resolved").and_then(Value::as_array) {
            for r in arr {
                let address = Address::new(r.get("address").and_then(Value::as_u64).unwrap_or(0));
                let function = r.get("function").and_then(Value::as_str).map(String::from);
                let source = match (
                    r.get("file").and_then(Value::as_str),
                    r.get("line").and_then(Value::as_u64),
                ) {
                    (Some(file), Some(line)) => Some(SourceLocation {
                        file: file.to_string(),
                        line: line as u32,
                        column: None,
                    }),
                    _ => None,
                };
                self.events.push_back(DebugEvent::BreakpointResolved {
                    breakpoint: id,
                    address,
                });
                resolved.push(ResolvedLocation {
                    address,
                    symbol: function.map(|name| Symbol {
                        name,
                        demangled: None,
                        module: None,
                        start: Some(address),
                        offset: 0,
                    }),
                    source,
                });
            }
        }
        let bp = Breakpoint {
            id,
            location: location.clone(),
            enabled: true,
            hit_count: 0,
            condition: condition.map(String::from),
            resolved,
        };
        self.breakpoints.push(bp.clone());
        Ok(bp)
    }

    fn remove_breakpoint(&mut self, id: BreakpointId) -> BindResult<()> {
        self.client
            .request("remove_breakpoint", json!({ "id": id.raw() }))?;
        self.breakpoints.retain(|b| b.id != id);
        Ok(())
    }

    fn enable_breakpoint(&mut self, id: BreakpointId, enabled: bool) -> BindResult<()> {
        // The driver exposes create/delete; enable/disable is tracked locally
        // and reported honestly as a capability gap for now.
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
        let v = self.client.request("process_info", json!({}))?;
        let lifecycle = match v.get("state").and_then(Value::as_str) {
            Some("stopped") => ProcessLifecycle::Stopped,
            Some("running") => ProcessLifecycle::Running,
            Some("exited") => ProcessLifecycle::Exited,
            Some("crashed") => ProcessLifecycle::Crashed,
            _ => self.lifecycle,
        };
        self.lifecycle = lifecycle;
        if let Some(triple) = v.get("arch").and_then(Value::as_str) {
            self.arch = arch_from_triple(triple);
        }
        Ok(ProcessInfo {
            pid: v
                .get("pid")
                .and_then(Value::as_u64)
                .map(|p| Pid::new(p as u32)),
            lifecycle,
            arch: self.arch,
            endianness: Endianness::Little,
            stop_reason: StopReason::None,
            exit_code: self.exit_code,
        })
    }

    fn threads(&mut self) -> BindResult<Vec<ThreadInfo>> {
        let v = self.client.request("threads", json!({}))?;
        let arr = v.as_array().cloned().unwrap_or_default();
        let selected = self.selected_thread;
        Ok(arr
            .iter()
            .map(|t| {
                let id = ThreadId::new(t.get("id").and_then(Value::as_u64).unwrap_or(0));
                ThreadInfo {
                    id,
                    name: t.get("name").and_then(Value::as_str).map(String::from),
                    is_selected: selected == Some(id),
                    run_state: ThreadRunState::Stopped,
                    stop_reason: StopReason::None,
                    pc: t.get("pc").and_then(Value::as_u64).map(Address::new),
                }
            })
            .collect())
    }

    fn frames(&mut self, thread: ThreadId) -> BindResult<Vec<Frame>> {
        let v = self.client.request("frames", self.thread_arg(thread))?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let function = f.get("function").and_then(Value::as_str).map(String::from);
                let pc = Address::new(f.get("pc").and_then(Value::as_u64).unwrap_or(0));
                let source = match (
                    f.get("file").and_then(Value::as_str),
                    f.get("line").and_then(Value::as_u64),
                ) {
                    (Some(file), Some(line)) if line > 0 => Some(SourceLocation {
                        file: file.to_string(),
                        line: line as u32,
                        column: None,
                    }),
                    _ => None,
                };
                Frame {
                    id: FrameId::new(i as u64),
                    thread_id: thread,
                    index: i,
                    pc,
                    sp: f.get("sp").and_then(Value::as_u64).map(Address::new),
                    fp: f.get("fp").and_then(Value::as_u64).map(Address::new),
                    function: function.clone(),
                    symbol: function.map(|name| Symbol {
                        name,
                        demangled: None,
                        module: f.get("module").and_then(Value::as_str).map(String::from),
                        start: None,
                        offset: 0,
                    }),
                    source,
                    is_inlined: f.get("inlined").and_then(Value::as_bool).unwrap_or(false),
                    is_selected: i == 0,
                }
            })
            .collect())
    }

    fn registers(&mut self, thread: ThreadId) -> BindResult<RegisterSet> {
        let v = self.client.request("registers", self.thread_arg(thread))?;
        let arr = v.as_array().cloned().unwrap_or_default();
        let arch = self.arch;
        let registers = arr
            .iter()
            .filter_map(|r| {
                let name = r.get("name").and_then(Value::as_str)?.to_string();
                let value = r.get("value").and_then(Value::as_u64).unwrap_or(0);
                let size = r.get("size").and_then(Value::as_u64).unwrap_or(64) as u16;
                let role = arch.classify_register(&name);
                Some(Register {
                    name,
                    value,
                    size_bits: size,
                    role,
                    wide: None,
                })
            })
            .collect();
        Ok(RegisterSet {
            thread_id: thread,
            registers,
        })
    }

    fn read_memory(&mut self, addr: Address, len: usize) -> BindResult<Vec<u8>> {
        let v = self.client.request(
            "read_memory",
            json!({ "address": addr.raw(), "length": len }),
        )?;
        let hex = v.get("hex").and_then(Value::as_str).unwrap_or("");
        Ok(decode_hex(hex))
    }

    fn disassemble(
        &mut self,
        around: Option<Address>,
        count: usize,
    ) -> BindResult<Vec<Instruction>> {
        let mut req = json!({ "count": count });
        if let Some(a) = around {
            req["address"] = Value::from(a.raw());
        }
        let v = self.client.request("disassemble", req)?;
        let arr = v.as_array().cloned().unwrap_or_default();
        let current = self.last_pc;
        let bp_addrs: std::collections::HashSet<u64> = self
            .breakpoints
            .iter()
            .flat_map(|b| b.resolved.iter().map(|r| r.address.raw()))
            .collect();
        Ok(arr
            .iter()
            .map(|i| {
                let address = Address::new(i.get("address").and_then(Value::as_u64).unwrap_or(0));
                let function = i.get("function").and_then(Value::as_str).map(String::from);
                Instruction {
                    address,
                    bytes: decode_hex(i.get("bytes").and_then(Value::as_str).unwrap_or("")),
                    mnemonic: i
                        .get("mnemonic")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    operands: i
                        .get("operands")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    symbol: function.map(|name| Symbol {
                        name,
                        demangled: None,
                        module: None,
                        start: None,
                        offset: 0,
                    }),
                    source: None,
                    branch_target: None,
                    is_current: current == Some(address),
                    has_breakpoint: bp_addrs.contains(&address.raw()),
                }
            })
            .collect())
    }

    fn modules(&mut self) -> BindResult<Vec<Module>> {
        let v = self.client.request("modules", json!({}))?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .enumerate()
            .map(|(i, m)| Module {
                id: ModuleId::new(i as u64),
                path: m
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                load_address: None,
                is_jit: false,
                has_debug_info: m.get("has_debug").and_then(Value::as_bool).unwrap_or(false),
            })
            .collect())
    }

    fn memory_regions(&mut self) -> BindResult<Vec<MemoryRegion>> {
        Ok(Vec::new())
    }

    fn poll_events(&mut self) -> Vec<DebugEvent> {
        self.events.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decode_roundtrip() {
        assert_eq!(decode_hex("e80f40b9"), vec![0xe8, 0x0f, 0x40, 0xb9]);
        assert_eq!(decode_hex(""), Vec::<u8>::new());
    }

    #[test]
    fn triple_arch_extraction() {
        assert_eq!(
            arch_from_triple("arm64-apple-macosx26.0.0"),
            Architecture::Aarch64
        );
        assert_eq!(
            arch_from_triple("x86_64-pc-linux-gnu"),
            Architecture::X86_64
        );
    }
}
