//! Rendering.
//!
//! Every function here is a pure projection of [`UiState`] onto a ratatui
//! [`Frame`]; it reads a borrowed snapshot and writes cells, nothing more. That
//! keeps rendering off the debugger lock and lets the whole UI be exercised
//! with ratatui's `TestBackend` (see the tests in `lib.rs`). All panels guard
//! against tiny areas and missing data so the app degrades rather than panics.

use bind_core::ProcessLifecycle;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{Mode, UiState};
use crate::palette::{self, LayoutMode, Panel};
use crate::theme::{Glyphs, Theme};

pub fn render(frame: &mut Frame, state: &UiState) {
    let theme = Theme::from_config(state.prefs.theme);
    let glyphs = Glyphs::new(state.prefs.unicode);
    let area = frame.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(6),    // main
            Constraint::Length(8), // timeline / analysis
            Constraint::Length(1), // status / palette
        ])
        .split(area);

    render_header(frame, rows[0], state, &theme);
    render_main(frame, rows[1], state, &theme, &glyphs);
    render_timeline_row(frame, rows[2], state, &theme, &glyphs);
    render_status(frame, rows[3], state, &theme);

    if state.mode == Mode::Help {
        render_help(frame, area, state, &theme);
    }
}

fn panel_block<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(theme.accent)
        } else {
            theme.dim()
        })
        .title(Span::styled(format!(" {title} "), theme.title(focused)))
}

fn render_header(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let p = &state.snapshot.process;
    let lifecycle = match p.lifecycle {
        ProcessLifecycle::Idle => "idle",
        ProcessLifecycle::Launching => "launching",
        ProcessLifecycle::Running => "running",
        ProcessLifecycle::Stopped => "stopped",
        ProcessLifecycle::Exited => "exited",
        ProcessLifecycle::Detached => "detached",
        ProcessLifecycle::Crashed => "crashed",
    };
    let pid = p.pid.map(|x| x.to_string()).unwrap_or_else(|| "-".into());
    let program = state.snapshot.program.as_deref().unwrap_or("(no target)");
    let line = Line::from(vec![
        Span::styled(
            "Bind",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(theme.accent),
        ),
        Span::raw("  "),
        Span::raw(program.to_string()),
        Span::raw("   pid "),
        Span::styled(pid, theme.dim()),
        Span::raw("   "),
        Span::styled(lifecycle, theme.current_line()),
        Span::raw("   "),
        Span::raw(p.arch.display_name().to_string()),
        Span::raw("   stop: "),
        Span::styled(p.stop_reason.short(), theme.dim()),
        Span::raw("   backend: "),
        Span::raw(state.snapshot.backend_name.clone()),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_main(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme, glyphs: &Glyphs) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(cols[0]);
    render_code(frame, left[0], state, theme, glyphs);
    render_stack(frame, left[1], state, theme);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(cols[1]);
    render_registers(frame, right[0], state, theme);
    render_memory(frame, right[1], state, theme);
}

fn render_code(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme, glyphs: &Glyphs) {
    let title = match state.layout {
        LayoutMode::Source => "Source",
        LayoutMode::Disassembly => "Disassembly",
        LayoutMode::Mixed => "Source / Disassembly",
    };
    let focused = state.focus == Panel::Code;
    let block = panel_block(title, focused, theme);
    if state.snapshot.disassembly.is_empty() {
        let hint = Paragraph::new("no code — launch or attach a target (: to open palette)")
            .style(theme.dim())
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(hint, area);
        return;
    }
    let items: Vec<ListItem> = state
        .snapshot
        .disassembly
        .iter()
        .map(|insn| {
            let marker = if insn.is_current {
                glyphs.current
            } else if insn.has_breakpoint {
                glyphs.breakpoint
            } else {
                " "
            };
            let src = insn
                .source
                .as_ref()
                .map(|s| format!("  {}:{}", s.file, s.line))
                .unwrap_or_default();
            let bytes: String = insn
                .bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("");
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), theme.error()),
                Span::styled(format!("{}  ", insn.address), theme.dim()),
                Span::styled(format!("{bytes:<8}  "), theme.dim()),
                Span::raw(format!("{:<7}", insn.mnemonic)),
                Span::raw(insn.operands.clone()),
                Span::styled(src, theme.dim()),
            ]);
            let style = if insn.is_current {
                theme.current_line()
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_registers(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let focused = state.focus == Panel::Registers;
    let block = panel_block("Registers", focused, theme);
    let Some(regs) = &state.snapshot.registers else {
        frame.render_widget(
            Paragraph::new("no registers (process not stopped)")
                .style(theme.dim())
                .block(block),
            area,
        );
        return;
    };
    let changed: std::collections::HashSet<&str> = state
        .snapshot
        .register_changes
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    let items: Vec<ListItem> = regs
        .registers
        .iter()
        .map(|r| {
            let is_changed = changed.contains(r.name.as_str());
            let mut spans = vec![
                Span::styled(format!("{:<5}", r.name), theme.dim()),
                Span::raw(" "),
                Span::styled(
                    r.hex(),
                    if is_changed {
                        theme.changed()
                    } else {
                        Style::default()
                    },
                ),
            ];
            if state.prefs.show_decimal {
                spans.push(Span::styled(format!("  ({})", r.value), theme.dim()));
            }
            if is_changed {
                spans.push(Span::styled("  *", theme.changed()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_stack(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let focused = state.focus == Panel::Stack;
    let block = panel_block("Stack / Frames", focused, theme);
    if state.snapshot.frames.is_empty() {
        frame.render_widget(
            Paragraph::new("no frames").style(theme.dim()).block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = state
        .snapshot
        .frames
        .iter()
        .map(|f| {
            let func = f.function.as_deref().unwrap_or("??");
            let src = f
                .source
                .as_ref()
                .map(|s| format!("  {}:{}", s.file, s.line))
                .unwrap_or_default();
            let inl = if f.is_inlined { " [inlined]" } else { "" };
            let line = Line::from(vec![
                Span::styled(format!("#{:<2} ", f.index), theme.dim()),
                Span::styled(format!("{} ", f.pc), theme.dim()),
                Span::raw(func.to_string()),
                Span::styled(inl.to_string(), theme.dim()),
                Span::styled(src, theme.dim()),
            ]);
            let style = if f.is_selected {
                theme.current_line()
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_memory(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let block = panel_block("Memory", false, theme);
    let Some(mem) = &state.memory else {
        frame.render_widget(
            Paragraph::new("`:memory <addr> [len]` to inspect")
                .style(theme.dim())
                .block(block),
            area,
        );
        return;
    };
    let lines = hex_dump(mem.addr.raw(), &mem.bytes, 8);
    let text: Vec<Line> = lines.into_iter().map(Line::from).collect();
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

/// Formats bytes as `addr: hex hex ... | ascii` rows.
pub fn hex_dump(base: u64, bytes: &[u8], width: usize) -> Vec<String> {
    let width = width.max(1);
    bytes
        .chunks(width)
        .enumerate()
        .map(|(i, chunk)| {
            let addr = base + (i * width) as u64;
            let hex: String = chunk
                .iter()
                .map(|b| format!("{b:02x} "))
                .collect::<String>();
            let ascii: String = chunk
                .iter()
                .map(|b| {
                    if b.is_ascii_graphic() || *b == b' ' {
                        *b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{addr:08x}: {hex:<width$}| {ascii}", width = width * 3)
        })
        .collect()
}

fn render_timeline_row(
    frame: &mut Frame,
    area: Rect,
    state: &UiState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);
    render_timeline(frame, cols[0], state, theme, glyphs);
    render_analysis(frame, cols[1], state, theme);
}

fn render_timeline(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme, glyphs: &Glyphs) {
    let focused = state.focus == Panel::Timeline;
    let dropped = state.trace_dropped();
    let total = state.trace_total();
    let rec = if state.trace_persisting() {
        " ●REC"
    } else {
        ""
    };
    let title = format!(
        "Timeline  ({} shown, {total} total{}{rec})",
        state.timeline.len(),
        if dropped > 0 {
            format!(", {dropped} dropped")
        } else {
            String::new()
        }
    );
    let block = panel_block(&title, focused, theme);
    let visible = area.height.saturating_sub(2) as usize;
    let start = state
        .selected_timeline
        .saturating_sub(visible.saturating_sub(1));
    let items: Vec<ListItem> = state
        .timeline
        .iter()
        .enumerate()
        .skip(start)
        .take(visible.max(1))
        .map(|(i, ev)| {
            let selected = i == state.selected_timeline;
            let marker = if selected {
                glyphs.current
            } else {
                glyphs.bullet
            };
            let detail = event_detail(&ev.event);
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), theme.dim()),
                Span::styled(format!("{:>5} ", ev.seq.raw()), theme.dim()),
                Span::styled(format!("{:<18}", ev.event.kind()), Style::default()),
                Span::styled(detail, theme.dim()),
            ]);
            let style = if selected {
                theme.current_line()
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn event_detail(ev: &bind_core::DebugEvent) -> String {
    use bind_core::DebugEvent::*;
    match ev {
        Stopped { reason, pc, .. } => format!(
            "{} @ {}",
            reason.short(),
            pc.map(|p| p.to_string()).unwrap_or_default()
        ),
        ProcessExited { code } => format!("code {code}"),
        BreakpointHit { breakpoint, .. } => format!("#{breakpoint}"),
        SignalReceived { signal, .. } => signal.clone(),
        InstructionStepped { to, .. } => to.to_string(),
        ModuleLoaded { module } | JitCodeLoaded { module } => module.path.clone(),
        Error { message } | Notice { message } => message.clone(),
        _ => String::new(),
    }
}

fn render_analysis(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let block = panel_block("Analysis", false, theme);
    if state.findings.is_empty() {
        frame.render_widget(
            Paragraph::new("analyses populate as the target runs")
                .style(theme.dim())
                .block(block),
            area,
        );
        return;
    }
    let lines: Vec<Line> = state
        .findings
        .iter()
        .map(|f| {
            Line::from(vec![
                Span::styled(format!("[{}] ", f.confidence.label()), theme.dim()),
                Span::raw(f.summary.clone()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

fn render_status(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("error: ", theme.error()),
                Span::raw(err.clone()),
            ])),
            area,
        );
        return;
    }
    if state.mode == Mode::Palette {
        let suggestions = palette::suggestions(&state.palette_input);
        let hint = suggestions
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("  ");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(":", theme.title(true)),
                Span::raw(state.palette_input.clone()),
                Span::styled("   ", Style::default()),
                Span::styled(hint, theme.dim()),
            ])),
            area,
        );
        return;
    }
    let hints =
        "c:continue  n:next  s:step  i:insn  o:finish  b:break  Tab:focus  ::cmd  ?:help  q:quit";
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{}  ", state.status), Style::default()),
            Span::styled(hints, theme.dim()),
        ])),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let w = area.width.min(64);
    let h = area.height.min(18);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    frame.render_widget(Clear, popup);
    let topic = state.help_topic.as_deref().unwrap_or("overview");
    let body = help_text(topic);
    let help_title = format!("Help — {topic}");
    let para = Paragraph::new(body)
        .block(panel_block(&help_title, true, theme))
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Left);
    frame.render_widget(para, popup);
}

fn help_text(topic: &str) -> Vec<Line<'static>> {
    let lines: Vec<&str> = match topic {
        "break" => vec![
            "break <symbol>            set a breakpoint by symbol",
            "break address 0x1000      set a breakpoint by address",
            "break file main.rs:42     set a breakpoint by source line",
            "delete-breakpoint <id>    remove a breakpoint",
        ],
        _ => vec![
            "Bind — terminal debugger & tracer",
            "",
            "Execution:  c continue   n next   s step-in   o finish   i instruction",
            "Navigate:   Tab/Shift-Tab focus   j/k or arrows move selection",
            "Palette:    : opens the typed command palette",
            "Commands:   break, delete-breakpoint, thread, frame, memory, goto,",
            "            trace start|stop, layout source|disassembly|mixed",
            "Other:      ? help    q quit    Ctrl-C force quit",
            "",
            "Press any key to dismiss.",
        ],
    };
    lines
        .into_iter()
        .map(|l| Line::from(l.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_dump_formats_rows() {
        let rows = hex_dump(0x1000, &[0x41, 0x42, 0x00, 0xff], 4);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("00001000: 41 42 00 ff"));
        assert!(rows[0].ends_with("| AB.."));
    }
}
