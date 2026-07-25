//! The JSON line protocol client for the LLDB SB-API driver.
//!
//! Owns the driver subprocess and does synchronous request/response RPC. A
//! background reader thread feeds responses over a channel so requests can time
//! out instead of hanging forever — important because live process control
//! under LLDB can block on OS-level developer-tools authorization (see
//! `docs/debugger-backend.md`). Timeouts become typed [`BindError`]s, never a
//! frozen UI.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

use bind_core::{BindError, BindResult};
use serde_json::Value;

/// Default per-request timeout. Static ops answer in milliseconds; the generous
/// bound only matters for control ops that may block on authorization.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct DriverClient {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<String>,
    _reader: JoinHandle<()>,
    next_id: u64,
    timeout: Duration,
}

impl DriverClient {
    /// Spawns `python_exe driver_path` with `PYTHONPATH=pythonpath`.
    pub fn spawn(
        python_exe: &str,
        driver_path: &std::path::Path,
        pythonpath: &str,
    ) -> BindResult<Self> {
        let mut child = Command::new(python_exe)
            .arg("-u")
            .arg(driver_path)
            .env("PYTHONPATH", pythonpath)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| BindError::Backend(format!("failed to spawn lldb driver: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BindError::Internal("driver stdin missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BindError::Internal("driver stdout missing".into()))?;

        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::Builder::new()
            .name("bind-lldb-reader".into())
            .spawn(move || {
                let mut lines = BufReader::new(stdout).lines();
                while let Some(Ok(line)) = lines.next() {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| BindError::Internal(format!("reader thread: {e}")))?;

        Ok(Self {
            child,
            stdin,
            responses: rx,
            _reader: reader,
            next_id: 1,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Sends one request and waits for its response, returning the `result`
    /// object (or a typed error).
    pub fn request(&mut self, op: &str, mut params: Value) -> BindResult<Value> {
        let id = self.next_id;
        self.next_id += 1;
        if !params.is_object() {
            params = serde_json::json!({});
        }
        params["id"] = Value::from(id);
        params["op"] = Value::from(op);

        let line = serde_json::to_string(&params)
            .map_err(|e| BindError::Internal(format!("serialize request: {e}")))?;
        writeln!(self.stdin, "{line}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| BindError::Backend(format!("write to driver: {e}")))?;

        match self.responses.recv_timeout(self.timeout) {
            Ok(resp_line) => Self::parse_response(id, &resp_line),
            Err(RecvTimeoutError::Timeout) => Err(BindError::Backend(format!(
                "lldb driver timed out after {:?} on op '{op}' \
                 (live process control may require developer-tools authorization)",
                self.timeout
            ))),
            Err(RecvTimeoutError::Disconnected) => {
                Err(BindError::Backend("lldb driver exited unexpectedly".into()))
            }
        }
    }

    fn parse_response(expected_id: u64, line: &str) -> BindResult<Value> {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| BindError::Backend(format!("bad driver response: {e}")))?;
        let got_id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
        if got_id != expected_id {
            return Err(BindError::Internal(format!(
                "driver response id mismatch: expected {expected_id}, got {got_id}"
            )));
        }
        if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            let msg = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown driver error")
                .to_string();
            Err(BindError::Backend(msg))
        }
    }

    /// Best-effort graceful shutdown.
    pub fn shutdown(&mut self) {
        let _ = writeln!(self.stdin, "{{\"id\":0,\"op\":\"shutdown\"}}");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

impl Drop for DriverClient {
    fn drop(&mut self) {
        self.shutdown();
        let _ = self.child.kill();
    }
}
