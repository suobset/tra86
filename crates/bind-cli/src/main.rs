//! `bind` — the command-line entry point.
//!
//! Parses arguments, initializes file-based logging (never stdout — the TUI
//! owns the terminal), selects a backend, spawns the debugger worker, and hands
//! off to the TUI. The `diag` subcommand runs the noninteractive smoke flow
//! instead.

mod diag;
mod logging;

use bind_core::{AttachSpec, Capabilities, Command, Pid, SessionId, SessionSnapshot, TargetSpec};
use bind_debugger::{DebugBackend, MockBackend, WorkerHandle};
use bind_lldb::LldbBackend;
use bind_storage::Preferences;
use bind_tui::UiState;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bind",
    version,
    about = "Bind — a terminal-native debugger, tracer, and runtime analysis environment"
)]
struct Cli {
    /// Program to launch (when no subcommand is given).
    program: Option<String>,

    /// Arguments passed to the target program (after `--`).
    #[arg(last = true)]
    args: Vec<String>,

    /// Use the deterministic in-memory mock backend instead of LLDB.
    #[arg(long)]
    mock: bool,

    /// Stop at the program entry point instead of the first breakpoint.
    #[arg(long)]
    stop_at_entry: bool,

    /// Restrict the UI to ASCII (no Unicode glyphs).
    #[arg(long)]
    no_unicode: bool,

    /// Record a trace to this path.
    #[arg(long)]
    trace: Option<String>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Attach to a running process by pid.
    Attach {
        pid: u32,
        #[arg(long)]
        mock: bool,
    },
    /// Open a core file for post-mortem inspection.
    Open { core: String, program: String },
    /// Run a noninteractive diagnostic/smoke flow and print a report.
    Diag {
        /// Program to launch (defaults to the mock backend when omitted).
        program: Option<String>,
        #[arg(long)]
        mock: bool,
        #[arg(long, default_value = "main")]
        breakpoint: String,
    },
}

fn select_backend(prefer_mock: bool) -> anyhow::Result<Box<dyn DebugBackend>> {
    if prefer_mock {
        return Ok(Box::new(MockBackend::default()));
    }
    if bind_lldb::is_available() {
        match LldbBackend::new() {
            Ok(b) => return Ok(Box::new(b)),
            Err(e) => {
                tracing::warn!("LLDB backend unavailable ({e}); falling back to mock");
                eprintln!("warning: LLDB backend unavailable ({e}); using mock backend");
            }
        }
    } else {
        eprintln!("warning: LLDB SB-API not detected; using mock backend (`--mock` to silence)");
    }
    Ok(Box::new(MockBackend::default()))
}

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();
    logging::init();

    match cli.command.take() {
        Some(Cmd::Diag {
            program,
            mock,
            breakpoint,
        }) => {
            let use_mock = mock || program.is_none();
            let backend = select_backend(use_mock)?;
            let program = program.unwrap_or_else(|| "mock".into());
            print!("{}", diag::run_diag(backend, &program, &breakpoint));
            Ok(())
        }
        Some(Cmd::Attach { pid, mock }) => {
            let backend = select_backend(mock)?;
            run_tui(backend, Some(AttachSpec::Pid(Pid::new(pid))), &cli)
        }
        Some(Cmd::Open { core, program }) => {
            let backend = select_backend(cli.mock)?;
            run_tui(backend, Some(AttachSpec::CoreFile { program, core }), &cli)
        }
        None => {
            let backend = select_backend(cli.mock)?;
            let attach = cli.program.clone().map(|program| {
                AttachSpec::Launch(TargetSpec {
                    program,
                    args: cli.args.clone(),
                    cwd: None,
                    env: vec![],
                    stop_at_entry: cli.stop_at_entry,
                })
            });
            run_tui(backend, attach, &cli)
        }
    }
}

fn run_tui(
    backend: Box<dyn DebugBackend>,
    attach: Option<AttachSpec>,
    cli: &Cli,
) -> anyhow::Result<()> {
    let backend_name = backend.name().to_string();
    let program = match &attach {
        Some(AttachSpec::Launch(t)) => Some(t.program.clone()),
        Some(AttachSpec::CoreFile { program, .. }) => Some(program.clone()),
        _ => None,
    };

    let worker = WorkerHandle::spawn(SessionId::new(1), backend);

    let mut snapshot = SessionSnapshot::empty(SessionId::new(1), backend_name, Capabilities::NONE);
    snapshot.program = program;

    let mut prefs = Preferences::default();
    if cli.no_unicode {
        prefs.unicode = false;
    }

    let mut state = UiState::new(snapshot, prefs);

    if let Some(spec) = attach {
        worker.send(Command::Attach(spec)).ok();
    }
    if let Some(path) = &cli.trace {
        state.status = format!("trace -> {path}");
        worker
            .send(Command::TraceStart {
                path: Some(path.clone()),
            })
            .ok();
    }

    // The terminal guard restores on any exit path, including panic.
    if let Err(e) = bind_tui::run_app(&mut state, &worker) {
        // run_app already restored the terminal before returning the error.
        anyhow::bail!("tui error: {e}");
    }
    worker.shutdown();
    Ok(())
}
