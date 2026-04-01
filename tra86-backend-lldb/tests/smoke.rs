use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;
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

fn integration_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
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

fn build_cpp_fixture_binary(
    name: &str,
    source_text: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = unique_fixture_dir();
    fs::create_dir_all(&dir)?;

    let source = dir.join(format!("{name}.cpp"));
    let binary = dir.join(name);
    fs::write(&source, source_text)?;

    let status = Command::new("c++")
        .arg("-std=c++17")
        .arg("-g")
        .arg("-O0")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .status()?;
    assert!(status.success(), "failed to compile C++ fixture binary");

    Ok(fs::canonicalize(binary)?)
}

fn spawn_ready_fixture(binary: &Path) -> Result<Child, Box<dyn std::error::Error>> {
    let mut child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or("fixture child stdout was not piped")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    assert!(
        line.contains("ready"),
        "fixture did not signal readiness, got: {line:?}"
    );

    Ok(child)
}

#[test]
fn lldb_backend_can_launch_and_inspect_real_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = integration_test_guard();

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
fn lldb_backend_resolves_cpp_symbols_and_source() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = integration_test_guard();

    if !tool_exists("lldb") || !tool_exists("c++") {
        eprintln!("skipping LLDB C++ symbol test because lldb or c++ is unavailable");
        return Ok(());
    }

    let binary = build_cpp_fixture_binary(
        "symbol_fixture",
        r#"#include <iostream>

static int leaf(int value) {
    return value * 3;
}

static int compute(int seed) {
    return leaf(seed + 4);
}

int main() {
    int result = compute(7);
    std::cout << "result=" << result << std::endl;
    return result;
}
"#,
    )?;
    let binary_str = binary.to_string_lossy().into_owned();

    let mut backend = LldbBackend::new();
    backend.open_target(&binary_str)?;
    backend.launch(LaunchRequest {
        target: TargetBinary {
            program: binary_str,
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        },
    })?;

    let threads = backend.list_threads()?;
    assert!(
        !threads.is_empty(),
        "expected launch to stop with a live thread"
    );
    let thread_id = threads[0].id;
    let frames = backend.list_frames(thread_id)?;
    assert!(!frames.is_empty(), "expected frames after launch");

    let frame_ip = frames[0].instruction_pointer;
    let symbol = backend.symbolicate(frame_ip)?;
    let source = backend.source_location(frame_ip)?;

    let symbol = symbol.expect("expected symbol lookup to return a symbol");
    let source = source.expect("expected source lookup to return a source location");
    assert!(
        !symbol.name.is_empty(),
        "expected a non-empty symbol name for the current frame"
    );
    assert!(
        symbol.name.contains("main"),
        "expected to resolve the main symbol, got {}",
        symbol.name
    );
    assert!(
        source.file.ends_with("symbol_fixture.cpp"),
        "expected C++ source file in lookup result, got {}",
        source.file
    );

    Ok(())
}

#[test]
fn lldb_backend_can_attach_to_live_cpp_process_and_list_threads(
) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = integration_test_guard();

    if !tool_exists("lldb") || !tool_exists("c++") {
        eprintln!("skipping LLDB attach test because lldb or c++ is unavailable");
        return Ok(());
    }

    let binary = build_cpp_fixture_binary(
        "threaded_fixture",
        r#"#include <atomic>
#include <chrono>
#include <iostream>
#include <thread>

static std::atomic<bool> keep_running{true};

static int busy_work(int value) {
    if (value <= 1) {
        return value;
    }
    return busy_work(value - 1) + 1;
}

int main() {
    std::thread worker([] {
        while (keep_running.load()) {
            std::this_thread::sleep_for(std::chrono::milliseconds(20));
        }
    });

    std::cout << "ready" << std::endl;
    std::cout.flush();

    for (int i = 0; i < 300; ++i) {
        busy_work(12);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }

    keep_running = false;
    worker.join();
    return 0;
}
"#,
    )?;

    let mut child = spawn_ready_fixture(&binary)?;
    thread::sleep(Duration::from_millis(200));

    let pid = child.id();
    let mut backend = LldbBackend::new();
    backend.attach(pid)?;

    let threads = backend.list_threads()?;
    assert!(
        threads.len() >= 2,
        "expected at least two threads after attach, got {}",
        threads.len()
    );

    let thread_id = threads
        .iter()
        .find(|thread| thread.is_current)
        .map(|thread| thread.id)
        .unwrap_or(threads[0].id);
    let frames = backend.list_frames(thread_id)?;
    assert!(
        !frames.is_empty(),
        "expected stack frames for the selected thread"
    );

    let registers = backend.read_registers(thread_id)?;
    assert!(
        !registers.registers.is_empty(),
        "expected registers to be readable after attach"
    );

    backend.detach()?;
    let _ = child.kill();
    let _ = child.wait();

    Ok(())
}
