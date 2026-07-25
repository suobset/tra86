//! Terminal lifecycle with guaranteed restoration.
//!
//! The terminal is a global resource: if Bind exits — cleanly, by error, or by
//! panic — while in raw mode / the alternate screen, it must be restored or the
//! user's shell is left broken. [`TerminalGuard`] restores on drop, and
//! [`install_panic_hook`] ensures restoration runs before a panic message is
//! printed.

use std::io::{self, Stdout};

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enters raw mode + alternate screen and returns a terminal plus a guard that
/// restores on drop.
pub fn enter() -> io::Result<(Tui, TerminalGuard)> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok((terminal, TerminalGuard { active: true }))
}

/// Restores the terminal to its normal state. Safe to call more than once.
pub fn restore() -> io::Result<()> {
    let mut out = io::stdout();
    let _ = execute!(out, LeaveAlternateScreen);
    disable_raw_mode()
}

/// Restores the terminal when dropped.
pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    /// Consume the guard without restoring (restoration already handled).
    pub fn disarm(mut self) {
        self.active = false;
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = restore();
        }
    }
}

/// Installs a panic hook that restores the terminal before the default hook
/// prints the panic, so a crash never leaves a garbled terminal.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        default(info);
    }));
}
