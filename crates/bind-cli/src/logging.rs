//! Logging setup.
//!
//! The TUI owns the terminal, so logs must never go to stdout/stderr while it
//! runs. Logging is therefore opt-in via the `BIND_LOG` environment variable
//! (a file path); when unset, Bind installs no subscriber and stays silent.
//! `RUST_LOG` controls verbosity as usual.

use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// Initializes file logging if `BIND_LOG` names a writable path. Idempotent and
/// never panics — a logging failure must not stop the debugger from starting.
pub fn init() {
    let Some(path) = std::env::var_os("BIND_LOG") else {
        return;
    };
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("warning: could not open BIND_LOG {path:?}: {e}");
            return;
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .try_init();
}
