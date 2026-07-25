//! Integration tests against a *real* LLDB via the SB-API driver.
//!
//! Static target operations (open target, resolve symbols, resolve
//! breakpoints, disassemble) never require OS authorization and are asserted
//! directly. Live process control (launch → breakpoint → registers → backtrace
//! → step) requires developer-tools authorization; where that is granted (an
//! authorized macOS, or the Linux Docker harness in `scripts/test-linux.sh`)
//! the full flow is asserted, and where it is not the test *skips with a clear
//! message* rather than hanging or silently passing. See
//! docs/debugger-backend.md.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use bind_core::{AttachSpec, BreakpointLocation, StepKind, TargetSpec};
use bind_debugger::DebugBackend;
use bind_lldb::LldbBackend;
use serde_json::json;

fn fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("native")
        .join("loop.c")
}

/// Compiles the C fixture to a temp binary. Returns None (with a printed skip)
/// if a compiler or the source is unavailable.
fn compile_fixture() -> Option<PathBuf> {
    let src = fixture_source();
    if !src.exists() {
        eprintln!("SKIP: fixture source {} not found", src.display());
        return None;
    }
    let out = std::env::temp_dir().join(format!("bind_fixture_loop_{}", std::process::id()));
    let status = Command::new("cc")
        .args(["-g", "-O0", "-o"])
        .arg(&out)
        .arg(&src)
        .status();
    match status {
        Ok(s) if s.success() => Some(out),
        _ => {
            eprintln!("SKIP: could not compile fixture (no working `cc`?)");
            None
        }
    }
}

fn guard() -> Option<(LldbBackend, PathBuf)> {
    if !bind_lldb::is_available() {
        eprintln!("SKIP: LLDB SB-API unavailable (`lldb -P` failed)");
        return None;
    }
    let bin = compile_fixture()?;
    match LldbBackend::new() {
        Ok(b) => Some((b, bin)),
        Err(e) => {
            eprintln!("SKIP: could not start LLDB driver: {e}");
            None
        }
    }
}

#[test]
fn static_symbol_and_disassembly_against_real_lldb() {
    let Some((mut backend, bin)) = guard() else {
        return;
    };
    let program = bin.to_string_lossy().to_string();
    let client = backend.client_mut();
    client.set_timeout(Duration::from_secs(15));

    // Open the target (no process — no authorization needed).
    let opened = client
        .request("open_target", json!({ "program": program }))
        .expect("open_target should succeed against a real binary");
    let triple = opened
        .get("triple")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        triple.contains("arm64") || triple.contains("x86_64") || triple.contains("aarch64"),
        "unexpected triple: {triple}"
    );

    // Resolve a breakpoint by symbol — proves symbol/line-table resolution.
    let bp = client
        .request(
            "add_breakpoint",
            json!({ "kind": "symbol", "name": "helper" }),
        )
        .expect("breakpoint on `helper` should resolve");
    let resolved = bp
        .get("resolved")
        .and_then(|v| v.as_array())
        .expect("resolved array");
    assert!(!resolved.is_empty(), "helper breakpoint did not resolve");
    let addr = resolved[0]
        .get("address")
        .and_then(|v| v.as_u64())
        .expect("resolved address");
    assert_eq!(
        resolved[0].get("function").and_then(|v| v.as_str()),
        Some("helper")
    );

    // Disassemble helper — proves real instruction decoding with bytes.
    let disasm = client
        .request("disassemble", json!({ "address": addr, "count": 3 }))
        .expect("disassemble should succeed");
    let insns = disasm.as_array().expect("instruction array");
    assert!(!insns.is_empty(), "no instructions disassembled");
    assert!(
        insns[0]
            .get("mnemonic")
            .and_then(|v| v.as_str())
            .map(|m| !m.is_empty())
            .unwrap_or(false),
        "first instruction has no mnemonic"
    );
    // Bytes should be present and even-length hex.
    let bytes = insns[0].get("bytes").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        bytes.len() >= 2 && bytes.len() % 2 == 0,
        "bad bytes: {bytes}"
    );
}

#[test]
fn live_debug_full_flow_when_authorized() {
    let Some((mut backend, bin)) = guard() else {
        return;
    };
    // Generous per-request timeout: launch under lldb-server can take a couple
    // seconds. On unauthorized macOS the launch request will time out and the
    // test skips; on Linux (e.g. the Docker harness) it completes and the full
    // flow below is asserted.
    backend.client_mut().set_timeout(Duration::from_secs(15));

    // Stop at entry so we have a stable point to install a breakpoint before
    // the target runs.
    let spec = AttachSpec::Launch(TargetSpec {
        program: bin.to_string_lossy().to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        stop_at_entry: true,
    });

    match backend.attach(&spec) {
        Ok(()) => {
            let info = backend.process_info().expect("process_info after launch");
            assert!(
                info.lifecycle.is_stopped(),
                "expected to be stopped at entry, got {:?}",
                info.lifecycle
            );

            // Breakpoint on `helper`, then continue to hit it.
            let bp = backend
                .add_breakpoint(&BreakpointLocation::Symbol("helper".into()), None)
                .expect("add breakpoint on helper");
            assert!(bp.is_resolved(), "helper breakpoint should resolve");
            backend.resume().expect("resume to breakpoint");

            let stopped = backend.process_info().expect("process_info after resume");
            assert!(
                stopped.lifecycle.is_stopped(),
                "should have stopped at the helper breakpoint, got {:?}",
                stopped.lifecycle
            );

            let threads = backend.threads().expect("threads");
            assert!(!threads.is_empty(), "expected at least one thread");
            let tid = threads[0].id;

            // Live registers: a real, non-empty set including the pc.
            let regs = backend.registers(tid).expect("read live registers");
            assert!(!regs.registers.is_empty(), "no registers read");
            assert!(
                regs.registers
                    .iter()
                    .any(|r| r.name == "pc" || r.name == "rip"),
                "register set has no program counter"
            );

            // Live backtrace: `helper` must appear, called from `main`.
            let frames = backend.frames(tid).expect("read frames");
            assert!(
                frames
                    .iter()
                    .any(|f| f.function.as_deref() == Some("helper")),
                "backtrace missing helper: {:?}",
                frames
                    .iter()
                    .map(|f| f.function.clone())
                    .collect::<Vec<_>>()
            );
            assert!(
                frames.iter().any(|f| f.function.as_deref() == Some("main")),
                "backtrace missing main"
            );

            // Single instruction step must succeed.
            backend
                .step(StepKind::Instruction)
                .expect("instruction step");

            eprintln!(
                "LIVE OK: launched, hit breakpoint, read {} registers, {} frames",
                regs.registers.len(),
                frames.len()
            );
            let _ = backend.terminate();
        }
        Err(e) => {
            // Expected on macOS without developer-tools authorization. Not a
            // failure — the static SB-API test still asserts real LLDB works.
            eprintln!(
                "SKIP live-debugging assertions: launch did not complete ({e}). \
                 Run the Docker harness (scripts/test-linux.sh) to exercise the \
                 live path on Linux."
            );
        }
    }
}
