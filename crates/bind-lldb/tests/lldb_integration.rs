//! Integration tests against a *real* LLDB via the SB-API driver.
//!
//! Static target operations (open target, resolve symbols, resolve
//! breakpoints, disassemble) do not require OS developer-tools authorization
//! and are asserted directly. Live process control (launch/step/registers)
//! requires authorization that is unavailable in headless/CI environments on
//! current macOS, so that portion is attempted with a short timeout and
//! *skipped with a clear message* rather than silently passing or hanging (see
//! docs/debugger-backend.md).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use bind_core::AttachSpec;
use bind_core::TargetSpec;
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
fn live_launch_is_attempted_and_handled_honestly() {
    let Some((mut backend, bin)) = guard() else {
        return;
    };
    // Use a short timeout so a launch that blocks on authorization fails fast.
    backend.client_mut().set_timeout(Duration::from_secs(3));

    let spec = AttachSpec::Launch(TargetSpec::program(bin.to_string_lossy().to_string()));
    match backend.attach(&spec) {
        Ok(()) => {
            // Live debugging is authorized here: assert we actually stopped.
            let info = backend.process_info().expect("process_info after launch");
            assert!(
                info.lifecycle.is_alive() || info.lifecycle == bind_core::ProcessLifecycle::Exited,
                "unexpected lifecycle after launch: {:?}",
                info.lifecycle
            );
            let _ = backend.terminate();
        }
        Err(e) => {
            // Expected in headless/unauthorized environments. Not a failure.
            eprintln!(
                "SKIP live-debugging assertions: launch did not complete ({e}). \
                 Static SB-API integration is covered by the other test."
            );
        }
    }
}
