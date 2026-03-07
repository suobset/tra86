use std::env;

use tra86_backend::{DebugBackend, LaunchRequest};
use tra86_backend_lldb::LldbBackend;
use tra86_core::TargetBinary;

fn main() -> anyhow::Result<()> {
    let program = env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: lldb_smoke <program-path>"))?;

    let mut backend = LldbBackend::new();
    backend.open_target(&program)?;
    let pre = backend.disassemble(None, 64)?;
    println!("pre-launch disassembly rows: {}", pre.len());

    backend.launch(LaunchRequest {
        target: TargetBinary {
            program: program.clone(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        },
    })?;

    let threads = backend.list_threads().unwrap_or_default();
    let thread_id = threads.first().map(|t| t.id).unwrap_or(1);
    println!("threads: {} (selected thread {})", threads.len(), thread_id);

    let regs = backend.read_registers(thread_id)?;
    println!("register count: {}", regs.registers.len());

    let dis = backend.disassemble(None, 64)?;
    println!("post-launch disassembly rows: {}", dis.len());

    backend.step_into()?;
    let dis2 = backend.disassemble(None, 16)?;
    println!("post-step disassembly rows: {}", dis2.len());

    Ok(())
}
