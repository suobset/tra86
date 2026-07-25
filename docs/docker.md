# Live LLDB testing with Docker

Live process control under LLDB requires OS debugging authorization. On macOS
that means `sudo DevToolsSecurity -enable` and, sometimes, a GUI prompt — awkward
for CI. A Linux container has **no such gate** (just the `SYS_PTRACE`
capability), so it is the reproducible way to exercise Bind's live LLDB backend
end to end.

## Quick start

```bash
scripts/test-linux.sh
```

This builds `docker/Dockerfile.linux-test` and runs `cargo test -p bind-lldb`
inside it with `--cap-add=SYS_PTRACE`. Expected output includes:

```
test static_symbol_and_disassembly_against_real_lldb ... ok
LIVE OK: launched, hit breakpoint, read N registers, M frames
test live_debug_full_flow_when_authorized ... ok
```

Pass custom arguments to override the default command:

```bash
scripts/test-linux.sh cargo test -p bind-lldb --test lldb_integration -- --nocapture
```

## What the image sets up

The base is `rust:1-slim-trixie` (Debian, so `apt install lldb python3-lldb`).
Two Debian LLDB-packaging quirks are repaired at build time so the SB-API driver
works:

1. **Dangling `_lldb` binding symlink.** The `_lldb.*.so` in the directory
   `lldb -P` reports is a symlink to a path that doesn't exist, so `import lldb`
   fails. The image repoints it at the real extension under `dist-packages`.
2. **`lldb-server` discovery.** When LLDB is embedded via the Python module it
   looks for `lldb-server-<full-version>` next to `liblldb` and in the install
   bindir; Debian installs it elsewhere. The image symlinks it into place.

Both are pure filesystem fixes baked into the image, so no runtime setup is
needed.

## Runtime flags explained

| Flag | Why |
|---|---|
| `--cap-add=SYS_PTRACE` | LLDB/`lldb-server` use `ptrace` to control the target. |
| `--security-opt seccomp=unconfined` | Avoids the default seccomp profile blocking ptrace-related syscalls. |
| `-v "$PWD":/src:ro` | Mounts the repo read-only; the source tree is never written. |
| `CARGO_TARGET_DIR=/tmp/bind-target` | Keeps build artifacts out of the mounted tree (and off the host's macOS `target/`). |
| `-v bind-cargo-registry:/usr/local/cargo/registry` | Caches the crate registry across runs. |
| `--memory=4g` | Conservative cap; `bind-lldb` doesn't pull in the TUI stack, so the build is light. |

## Notes

- Only `bind-lldb` is built here — it's all that's needed to exercise the live
  backend, and it avoids compiling ratatui/crossterm.
- To run the **whole** suite in Linux instead:
  `scripts/test-linux.sh cargo test --workspace` (heavier; builds the TUI too).
- Ptrace inside Docker Desktop on Apple Silicon runs in the Linux VM; the host
  is unaffected.
