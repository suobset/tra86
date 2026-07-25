//! The interactive event loop.
//!
//! Ties the terminal, the [`UiState`], and the debugger [`WorkerHandle`]
//! together: render, read input, translate to typed commands, drain worker
//! updates, repeat. Rendering happens only when something changed (input,
//! worker update, or resize), not on a busy timer, so an idle Bind does not
//! spin the CPU or redraw needlessly.
//!
//! Input is read through the [`InputSource`] trait so the whole loop can be
//! driven by a scripted source in tests (against a real worker + mock backend),
//! and the loop is generic over the ratatui [`Backend`] so it can render into a
//! `TestBackend` off a real terminal.

use std::time::Duration;

use bind_debugger::WorkerHandle;
use crossterm::event::{self, Event};
use ratatui::backend::Backend;
use ratatui::Terminal;

use crate::app::UiState;
use crate::terminal;

/// An abstract source of terminal input events. The real implementation reads
/// crossterm; tests supply a scripted one.
pub trait InputSource {
    /// Returns the next event, or `None` if `timeout` elapses with no input.
    fn next_event(&mut self, timeout: Duration) -> std::io::Result<Option<Event>>;
}

/// The real crossterm-backed input source.
pub struct CrosstermInput;

impl InputSource for CrosstermInput {
    fn next_event(&mut self, timeout: Duration) -> std::io::Result<Option<Event>> {
        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }
}

/// A scripted input source for tests: yields queued events, then `None` forever.
pub struct ScriptedInput {
    events: std::collections::VecDeque<Event>,
}

impl ScriptedInput {
    pub fn new(events: impl IntoIterator<Item = Event>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl InputSource for ScriptedInput {
    fn next_event(&mut self, _timeout: Duration) -> std::io::Result<Option<Event>> {
        Ok(self.events.pop_front())
    }
}

/// Runs the UI loop until the user quits. Generic over the terminal backend and
/// input source so it is fully testable. Assumes the terminal is already set up.
pub fn run<B, I>(
    terminal: &mut Terminal<B>,
    state: &mut UiState,
    worker: &WorkerHandle,
    input: &mut I,
) -> std::io::Result<()>
where
    B: Backend,
    I: InputSource,
{
    terminal.draw(|f| crate::view::render(f, state))?;

    loop {
        let mut dirty = false;

        if let Some(evt) = input.next_event(Duration::from_millis(50))? {
            match evt {
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
    let mut input = CrosstermInput;
    let result = run(&mut term, state, worker, &mut input);
    drop(term);
    guard.disarm();
    terminal::restore()?;
    result
}
