use std::env;
use std::io::{self, Write};

use anyhow::{anyhow, bail, Context};
use tra86_backend::{DebugBackend, LaunchRequest};
use tra86_backend_lldb::LldbBackend;
use tra86_core::{Address, Breakpoint, BreakpointLocation, StopReason, TargetBinary, ThreadId};

fn main() -> anyhow::Result<()> {
    let startup = StartupAction::parse(env::args().skip(1).collect())?;
    let mut cli = CliSession::new();

    cli.print_banner();
    cli.apply_startup(startup)?;
    cli.repl()
}

#[derive(Debug)]
enum StartupAction {
    None,
    Open(String),
    Launch(TargetBinary),
    Attach(u32),
}

impl StartupAction {
    fn parse(args: Vec<String>) -> anyhow::Result<Self> {
        if args.is_empty() {
            return Ok(Self::None);
        }

        match args[0].as_str() {
            "help" | "--help" | "-h" => {
                print_startup_help();
                std::process::exit(0);
            }
            "open" => {
                let program = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| anyhow!("usage: tra86_cli open <program>"))?;
                Ok(Self::Open(program))
            }
            "launch" => {
                let (program, rest) = parse_program_and_args(&args[1..])?;
                Ok(Self::Launch(TargetBinary {
                    program,
                    args: rest,
                    cwd: None,
                    env: Vec::new(),
                }))
            }
            "attach" => {
                let pid = args
                    .get(1)
                    .ok_or_else(|| anyhow!("usage: tra86_cli attach <pid>"))?
                    .parse::<u32>()
                    .context("attach pid must be an integer")?;
                Ok(Self::Attach(pid))
            }
            other => bail!(
                "unknown startup command `{other}`\ntry `tra86_cli --help` for usage"
            ),
        }
    }
}

struct CliSession {
    backend: LldbBackend,
    target: Option<TargetBinary>,
    attached_pid: Option<u32>,
    last_stop_reason: StopReason,
    breakpoints: Vec<Breakpoint>,
}

impl CliSession {
    fn new() -> Self {
        Self {
            backend: LldbBackend::new(),
            target: None,
            attached_pid: None,
            last_stop_reason: StopReason::None,
            breakpoints: Vec::new(),
        }
    }

    fn print_banner(&self) {
        println!("tra86 CLI");
        println!("type `help` for commands, `quit` to exit");
    }

    fn apply_startup(&mut self, startup: StartupAction) -> anyhow::Result<()> {
        match startup {
            StartupAction::None => Ok(()),
            StartupAction::Open(program) => self.open(program),
            StartupAction::Launch(target) => self.launch(target),
            StartupAction::Attach(pid) => self.attach(pid),
        }
    }

    fn repl(&mut self) -> anyhow::Result<()> {
        let stdin = io::stdin();

        loop {
            print!("tra86> ");
            io::stdout().flush().context("failed to flush prompt")?;

            let mut line = String::new();
            if stdin.read_line(&mut line)? == 0 {
                println!();
                break;
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match self.handle_line(line) {
                Ok(ControlFlow::Continue) => {}
                Ok(ControlFlow::Exit) => break,
                Err(err) => eprintln!("error: {err}"),
            }
        }

        Ok(())
    }

    fn handle_line(&mut self, line: &str) -> anyhow::Result<ControlFlow> {
        let tokens = tokenize(line)?;
        if tokens.is_empty() {
            return Ok(ControlFlow::Continue);
        }

        match tokens[0].as_str() {
            "help" => {
                print_repl_help();
            }
            "open" => {
                let program = require_arg(&tokens, 1, "usage: open <program>")?;
                self.open(program.to_string())?;
            }
            "launch" => {
                let (program, args) = parse_program_and_args(&tokens[1..])?;
                self.launch(TargetBinary {
                    program,
                    args,
                    cwd: None,
                    env: Vec::new(),
                })?;
            }
            "attach" => {
                let pid = require_arg(&tokens, 1, "usage: attach <pid>")?
                    .parse::<u32>()
                    .context("attach pid must be an integer")?;
                self.attach(pid)?;
            }
            "continue" | "c" => {
                self.backend.continue_exec()?;
                println!("continued target");
                self.refresh_summary()?;
            }
            "pause" => {
                self.backend.pause()?;
                self.refresh_summary()?;
            }
            "step" | "si" => {
                self.backend.step_into()?;
                self.refresh_summary()?;
            }
            "next" | "n" => {
                self.backend.step_over()?;
                self.refresh_summary()?;
            }
            "finish" | "out" => {
                self.backend.step_out()?;
                self.refresh_summary()?;
            }
            "threads" => {
                self.print_threads()?;
            }
            "frames" => {
                let thread_id = self.resolve_thread_arg(tokens.get(1))?;
                self.print_frames(thread_id)?;
            }
            "regs" | "registers" => {
                let thread_id = self.resolve_thread_arg(tokens.get(1))?;
                self.print_registers(thread_id)?;
            }
            "disasm" => {
                let count = tokens
                    .get(1)
                    .map(|value| value.parse::<usize>())
                    .transpose()
                    .context("disasm count must be an integer")?
                    .unwrap_or(16);
                let address = tokens
                    .get(2)
                    .map(|value| parse_address(value))
                    .transpose()?;
                self.print_disassembly(address, count)?;
            }
            "mem" | "memory" => {
                let address = parse_address(require_arg(&tokens, 1, "usage: mem <addr> [len]")?)?;
                let length = tokens
                    .get(2)
                    .map(|value| value.parse::<usize>())
                    .transpose()
                    .context("memory length must be an integer")?
                    .unwrap_or(64);
                self.print_memory(address, length)?;
            }
            "break" | "b" => {
                let address = parse_address(require_arg(&tokens, 1, "usage: break <addr>")?)?;
                let breakpoint = self
                    .backend
                    .set_breakpoint(BreakpointLocation::Address(address))?;
                self.breakpoints.push(breakpoint.clone());
                println!("breakpoint {} set at {}", breakpoint.id, format_address(address));
            }
            "delete" => {
                let id = require_arg(&tokens, 1, "usage: delete <breakpoint-id>")?
                    .parse::<u64>()
                    .context("breakpoint id must be an integer")?;
                self.backend.remove_breakpoint(id)?;
                self.breakpoints.retain(|breakpoint| breakpoint.id != id);
                println!("removed breakpoint {id}");
            }
            "breakpoints" | "bp" => {
                self.print_breakpoints();
            }
            "symbols" | "symbol" => {
                let address =
                    parse_address(require_arg(&tokens, 1, "usage: symbols <addr>")?)?;
                match self.backend.symbolicate(address)? {
                    Some(symbol) => {
                        let module = symbol.module.unwrap_or_else(|| "-".to_string());
                        println!(
                            "{} {} + 0x{:x}",
                            format_address(address),
                            module,
                            symbol.offset
                        );
                        println!("{}", symbol.name);
                    }
                    None => println!("no symbol for {}", format_address(address)),
                }
            }
            "source" => {
                let address = parse_address(require_arg(&tokens, 1, "usage: source <addr>")?)?;
                match self.backend.source_location(address)? {
                    Some(source) => {
                        println!(
                            "{}:{}:{}",
                            source.file,
                            source.line,
                            source.column.unwrap_or(0)
                        );
                    }
                    None => println!("no source location for {}", format_address(address)),
                }
            }
            "status" => {
                self.refresh_summary()?;
            }
            "refresh" => {
                self.refresh_summary()?;
                self.print_threads()?;
            }
            "detach" => {
                self.backend.detach()?;
                self.attached_pid = None;
                self.last_stop_reason = StopReason::Detached;
                println!("detached from process");
            }
            "kill" | "stop" => {
                self.backend.kill()?;
                self.attached_pid = None;
                self.last_stop_reason = StopReason::Exited(0);
                println!("killed target");
            }
            "quit" | "exit" => return Ok(ControlFlow::Exit),
            other => bail!("unknown command `{other}`; try `help`"),
        }

        Ok(ControlFlow::Continue)
    }

    fn open(&mut self, program: String) -> anyhow::Result<()> {
        self.backend.open_target(&program)?;
        self.target = Some(TargetBinary {
            program: program.clone(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        });
        self.attached_pid = None;
        self.breakpoints.clear();
        println!("loaded target {program}");
        self.print_disassembly(None, 12)
    }

    fn launch(&mut self, target: TargetBinary) -> anyhow::Result<()> {
        self.backend.launch(LaunchRequest {
            target: target.clone(),
        })?;
        self.target = Some(target.clone());
        self.attached_pid = None;
        self.breakpoints.clear();
        println!("launched {}", target.program);
        self.refresh_summary()
    }

    fn attach(&mut self, pid: u32) -> anyhow::Result<()> {
        self.backend.attach(pid)?;
        self.target = None;
        self.attached_pid = Some(pid);
        self.breakpoints.clear();
        println!("attached to pid {pid}");
        self.refresh_summary()
    }

    fn refresh_summary(&mut self) -> anyhow::Result<()> {
        if self.target.is_none() && self.attached_pid.is_none() {
            println!("no active target");
            return Ok(());
        }

        self.last_stop_reason = self.backend.current_stop_reason()?;
        let threads = self.backend.list_threads().unwrap_or_default();
        let thread_count = threads.len();
        let current_thread = threads
            .iter()
            .find(|thread| thread.is_current)
            .map(|thread| thread.id);

        match (&self.target, self.attached_pid) {
            (Some(target), _) => println!("target: {}", target.program),
            (None, Some(pid)) => println!("attached pid: {pid}"),
            (None, None) => println!("no active target"),
        }
        println!("stop reason: {:?}", self.last_stop_reason);
        println!("threads: {thread_count}");
        if let Some(thread_id) = current_thread {
            println!("current thread: {thread_id}");
        }

        Ok(())
    }

    fn print_threads(&mut self) -> anyhow::Result<()> {
        let threads = self.backend.list_threads()?;
        if threads.is_empty() {
            println!("no threads");
            return Ok(());
        }

        for thread in threads {
            let marker = if thread.is_current { "*" } else { " " };
            let ip = thread
                .instruction_pointer
                .map(format_address)
                .unwrap_or_else(|| "-".to_string());
            let name = thread.name.unwrap_or_else(|| "-".to_string());
            println!(
                "{marker} tid={} status={:?} stop={:?} ip={} name={}",
                thread.id, thread.status, thread.stop_reason, ip, name
            );
        }

        Ok(())
    }

    fn print_frames(&mut self, thread_id: ThreadId) -> anyhow::Result<()> {
        let frames = self.backend.list_frames(thread_id)?;
        if frames.is_empty() {
            println!("no frames for thread {thread_id}");
            return Ok(());
        }

        for frame in frames {
            let function = frame.function.unwrap_or_else(|| "<unknown>".to_string());
            let source = frame
                .source
                .map(|source| format!("{}:{}", source.file, source.line))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "#{} ip={} sp={} fp={} {} {}",
                frame.index,
                format_address(frame.instruction_pointer),
                frame
                    .stack_pointer
                    .map(format_address)
                    .unwrap_or_else(|| "-".to_string()),
                frame
                    .frame_pointer
                    .map(format_address)
                    .unwrap_or_else(|| "-".to_string()),
                function,
                source
            );
        }

        Ok(())
    }

    fn print_registers(&mut self, thread_id: ThreadId) -> anyhow::Result<()> {
        let registers = self.backend.read_registers(thread_id)?;
        if registers.registers.is_empty() {
            println!("no registers for thread {thread_id}");
            return Ok(());
        }

        for register in registers.registers {
            println!("{:<8} {}", register.name, register.value);
        }

        Ok(())
    }

    fn print_disassembly(&mut self, address: Option<Address>, count: usize) -> anyhow::Result<()> {
        let disassembly = self.backend.disassemble(address, count)?;
        if disassembly.is_empty() {
            println!("no disassembly available");
            return Ok(());
        }

        for line in disassembly {
            let current = if line.is_current { "=>" } else { "  " };
            let function = line.function.unwrap_or_default();
            if function.is_empty() {
                println!(
                    "{current} {} {:<8} {}",
                    format_address(line.address),
                    line.mnemonic,
                    line.operands
                );
            } else {
                println!(
                    "{current} {} {:<8} {} ; {}",
                    format_address(line.address),
                    line.mnemonic,
                    line.operands,
                    function
                );
            }
        }

        Ok(())
    }

    fn print_memory(&mut self, address: Address, length: usize) -> anyhow::Result<()> {
        let bytes = self.backend.read_memory(address, length)?;
        if bytes.is_empty() {
            println!("no bytes read");
            return Ok(());
        }

        for (offset, chunk) in bytes.chunks(16).enumerate() {
            let row_addr = address + (offset * 16) as u64;
            let hex = chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii = chunk
                .iter()
                .map(|byte| match byte {
                    0x20..=0x7e => char::from(*byte),
                    _ => '.',
                })
                .collect::<String>();
            println!("{}  {:<47}  {}", format_address(row_addr), hex, ascii);
        }

        Ok(())
    }

    fn print_breakpoints(&self) {
        if self.breakpoints.is_empty() {
            println!("no breakpoints");
            return;
        }

        for breakpoint in &self.breakpoints {
            match breakpoint.location {
                BreakpointLocation::Address(address) => {
                    println!("{} {}", breakpoint.id, format_address(address));
                }
                BreakpointLocation::Symbol(ref symbol) => {
                    println!("{} {}", breakpoint.id, symbol);
                }
            }
        }
    }

    fn resolve_thread_arg(&mut self, arg: Option<&String>) -> anyhow::Result<ThreadId> {
        if let Some(value) = arg {
            return value
                .parse::<ThreadId>()
                .context("thread id must be an integer");
        }

        let threads = self.backend.list_threads()?;
        threads
            .iter()
            .find(|thread| thread.is_current)
            .or_else(|| threads.first())
            .map(|thread| thread.id)
            .ok_or_else(|| anyhow!("no thread available"))
    }
}

enum ControlFlow {
    Continue,
    Exit,
}

fn parse_program_and_args(tokens: &[String]) -> anyhow::Result<(String, Vec<String>)> {
    let Some(program) = tokens.first() else {
        bail!("usage: launch <program> [-- args...]");
    };

    if let Some(separator_index) = tokens.iter().position(|token| token == "--") {
        let args = tokens[separator_index + 1..].to_vec();
        Ok((program.clone(), args))
    } else {
        Ok((program.clone(), tokens[1..].to_vec()))
    }
}

fn require_arg<'a>(tokens: &'a [String], index: usize, usage: &str) -> anyhow::Result<&'a str> {
    tokens
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| anyhow!(usage.to_string()))
}

fn parse_address(input: &str) -> anyhow::Result<Address> {
    let trimmed = input.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).context("address must be valid hex")
    } else {
        trimmed.parse::<u64>().context("address must be valid integer")
    }
}

fn format_address(address: Address) -> String {
    format!("0x{address:016x}")
}

fn tokenize(input: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) if ch == '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None if ch == '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            None => current.push(ch),
        }
    }

    if quote.is_some() {
        bail!("unterminated quoted string");
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn print_startup_help() {
    println!("usage:");
    println!("  tra86_cli");
    println!("  tra86_cli open <program>");
    println!("  tra86_cli launch <program> [-- args...]");
    println!("  tra86_cli attach <pid>");
}

fn print_repl_help() {
    println!("commands:");
    println!("  help                              show this help");
    println!("  open <program>                    load a target without launching");
    println!("  launch <program> [-- args...]     launch and stop at entry");
    println!("  attach <pid>                      attach to a running process");
    println!("  continue | c                      continue execution");
    println!("  pause                             interrupt execution");
    println!("  step | si                         single-step into");
    println!("  next | n                          step over");
    println!("  finish | out                      step out");
    println!("  status                            show current summary");
    println!("  refresh                           refresh summary and threads");
    println!("  threads                           list threads");
    println!("  frames [thread_id]                list frames");
    println!("  regs [thread_id]                  list registers");
    println!("  disasm [count] [addr]             show disassembly");
    println!("  mem <addr> [len]                  dump memory");
    println!("  break <addr>                      set address breakpoint");
    println!("  delete <breakpoint-id>            remove breakpoint");
    println!("  breakpoints | bp                  list breakpoints");
    println!("  symbols <addr>                    resolve symbol");
    println!("  source <addr>                     resolve source location");
    println!("  detach                            detach from process");
    println!("  kill | stop                       kill target process");
    println!("  quit | exit                       leave the CLI");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_supports_quotes_and_escapes() {
        let tokens =
            tokenize("launch \"./target with spaces\" -- \"hello world\" plain\\ arg").unwrap();
        assert_eq!(
            tokens,
            vec![
                "launch",
                "./target with spaces",
                "--",
                "hello world",
                "plain arg",
            ]
        );
    }

    #[test]
    fn parse_address_supports_hex_and_decimal() {
        assert_eq!(parse_address("0x10").unwrap(), 16);
        assert_eq!(parse_address("42").unwrap(), 42);
    }

    #[test]
    fn startup_launch_parses_tail_args() {
        let startup = StartupAction::parse(vec![
            "launch".to_string(),
            "./demo".to_string(),
            "--".to_string(),
            "one".to_string(),
            "two".to_string(),
        ])
        .unwrap();

        match startup {
            StartupAction::Launch(target) => {
                assert_eq!(target.program, "./demo");
                assert_eq!(target.args, vec!["one", "two"]);
            }
            _ => panic!("expected launch startup action"),
        }
    }
}
