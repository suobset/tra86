//! The interactive event loop.
//!
//! Ties the terminal, the [`UiState`], and the debugger [`WorkerHandle`]
//! together: render, read input, translate to typed commands, drain worker
//! updates, repeat. Rendering happens only when something changed (input,
//! worker update, or resize), not on a busy timer, so an idle Bind does not
//! spin the CPU or redraw needlessly.

use std::time::Duration;

use bind_debugger::WorkerHandle;
use crossterm::event::{self, Event};

use crate::app::UiState;
use crate::terminal::{self, Tui};

/// Runs the UI loop until the user quits. Assumes the terminal is already in
/// raw/alternate mode (see [`terminal::enter`]).
pub fn run(terminal: &mut Tui, state: &mut UiState, worker: &WorkerHandle) -> std::io::Result<()> {
    // Initial paint.
    terminal.draw(|f| crate::view::render(f, state))?;

    loop {
        let mut dirty = false;

        // Input, with a short poll so worker updates are picked up promptly.
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                    for cmd in state.handle_key(key) {
                        if worker.send(cmd).is_err() {
                            state.error = Some("debugger worker disconnected".into());
                        }
                    }
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }

        // Fold any worker updates.
        for update in worker.drain() {
            state.apply_update(update);
            dirty = true;
        }

        if state.should_quit {
            break;
        }
        if dirty {
            terminal.draw(|f| crate::view::render(f, state))?;
        }
    }
    Ok(())
}

/// Convenience entry point: sets up the terminal (panic-safe), runs, restores.
pub fn run_app(state: &mut UiState, worker: &WorkerHandle) -> std::io::Result<()> {
    terminal::install_panic_hook();
    let (mut term, guard) = terminal::enter()?;
    let result = run(&mut term, state, worker);
    drop(term);
    guard.disarm();
    terminal::restore()?;
    result
}
