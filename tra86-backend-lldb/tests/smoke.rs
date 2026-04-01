use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use tra86_backend::{DebugBackend, LaunchRequest};
use tra86_backend_lldb::LldbBackend;
use tra86_core::{BreakpointLocation, TargetBinary};

fn tool_exists(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn unique_fixture_dir() -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_millis();
    let mut dir = std::env::temp_dir();
    dir.push(format!("tra86-lldb-smoke-{}-{millis}", std::process::id()));
    dir
}

fn build_fixture_binary_with_sleep(
    sleep_seconds: u64,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = unique_fixture_dir();
    fs::create_dir_all(&dir)?;

    let source = dir.join("fixture.c");
    let binary = dir.join("fixture");
    fs::write(
        &source,
        r#"#include <stdio.h>
#include <unistd.h>

static int twice(int value) {
    return value * 2;
}

int main(int argc, char **argv) {
    int computed = twice(argc + 2);
    printf("computed=%d\n", computed);
    fflush(stdout);
    sleep(SLEEP_SECONDS);
    return computed;
}
"#
        .replace("SLEEP_SECONDS", &sleep_seconds.to_string()),
    )?;

    let status = Command::new("cc")
        .arg("-g")
        .arg("-O0")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .status()?;
    assert!(status.success(), "failed to compile fixture binary");

    Ok(fs::canonicalize(binary)?)
}

fn build_fixture_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    build_fixture_binary_with_sleep(1)
}

#[test]
fn lldb_backend_can_launch_and_inspect_real_fixture() -> Result<(), Box<dyn std::error::Error>> {
    if !tool_exists("lldb") || !Path::new("/usr/bin/cc").exists() {
        eprintln!("skipping LLDB smoke test because lldb or cc is unavailable");
        return Ok(());
    }

    let binary = build_fixture_binary()?;
    let binary_str = binary.to_string_lossy().into_owned();

    let mut backend = LldbBackend::new();
    backend.open_target(&binary_str)?;

    let prelaunch_disassembly = backend.disassemble(None, 32)?;
    assert!(
        !prelaunch_disassembly.is_empty(),
        "expected disassembly before launch"
    );

    backend.launch(LaunchRequest {
        target: TargetBinary {
            program: binary_str.clone(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        },
    })?;

    let threads = backend.list_threads()?;
    assert!(!threads.is_empty(), "expected at least one thread");
    let thread_id = threads[0].id;

    let registers = backend.read_registers(thread_id)?;
    assert!(
        registers.registers.len() >= 3,
        "expected at least a few registers, got {}",
        registers.registers.len()
    );

    let current_ip = backend.current_instruction(thread_id)?;
    assert!(
        current_ip.is_some(),
        "expected a current instruction pointer"
    );

    let disassembly = backend.disassemble(None, 32)?;
    assert!(!disassembly.is_empty(), "expected post-launch disassembly");

    let frames = backend.list_frames(thread_id)?;
    assert!(!frames.is_empty(), "expected at least one frame");

    let memory = backend.read_memory(current_ip.unwrap_or(disassembly[0].address), 32)?;
    assert!(!memory.is_empty(), "expected a memory read result");

    let breakpoint = backend.set_breakpoint(BreakpointLocation::Address(disassembly[0].address))?;
    backend.remove_breakpoint(breakpoint.id)?;

    backend.step_into()?;
    let stepped_disassembly = backend.disassemble(None, 16)?;
    assert!(
        !stepped_disassembly.is_empty(),
        "expected disassembly after step"
    );

    Ok(())
}

#[test]
fn lldb_interrupt_handle_can_pause_running_target() -> Result<(), Box<dyn std::error::Error>> {
    if !tool_exists("lldb") || !Path::new("/usr/bin/cc").exists() {
        eprintln!("skipping LLDB interrupt test because lldb or cc is unavailable");
        return Ok(());
    }

    let binary = build_fixture_binary_with_sleep(10)?;
    let binary_str = binary.to_string_lossy().into_owned();
    let backend = Arc::new(Mutex::new(LldbBackend::new()));

    {
        let mut backend = backend.lock().expect("backend mutex should lock");
        backend.open_target(&binary_str)?;
        backend.launch(LaunchRequest {
            target: TargetBinary {
                program: binary_str.clone(),
                args: Vec::new(),
                cwd: None,
                env: Vec::new(),
            },
        })?;
    }

    let control = backend
        .lock()
        .expect("backend mutex should lock")
        .control_handle()
        .expect("lldb backend should expose a control handle");

    let continue_started = Instant::now();
    let continue_thread = {
        let backend = Arc::clone(&backend);
        thread::spawn(move || -> Result<(), String> {
            let mut backend = backend
                .lock()
                .map_err(|_| "backend mutex poisoned".to_string())?;
            backend.continue_exec().map_err(|err| err.to_string())
        })
    };

    thread::sleep(Duration::from_millis(500));
    control.interrupt()?;

    let continue_result = continue_thread
        .join()
        .expect("continue thread should not panic");
    assert!(
        continue_result.is_ok(),
        "continue after interrupt should succeed: {:?}",
        continue_result.err()
    );
    assert!(
        continue_started.elapsed() < Duration::from_secs(6),
        "interrupt should stop a blocking continue well before the fixture exits"
    );

    let mut backend = backend.lock().expect("backend mutex should lock");
    let threads = backend.list_threads()?;
    assert!(
        !threads.is_empty(),
        "expected debugger to remain attached after interrupt"
    );
    let thread_id = threads[0].id;
    let registers = backend.read_registers(thread_id)?;
    assert!(
        !registers.registers.is_empty(),
        "expected registers to remain readable after interrupt"
    );

    Ok(())
}
