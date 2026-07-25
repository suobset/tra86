//! Visual styling.
//!
//! A restrained, mostly-monochrome palette that also works when the terminal
//! has no color and when Unicode glyphs are unavailable (the [`Glyphs`] set
//! swaps to ASCII). Nothing here assumes a specific terminal background.

use bind_storage::Theme as ConfigTheme;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub dim: Color,
    pub changed: Color,
    pub error: Color,
    pub current: Color,
    pub monochrome: bool,
}

impl Theme {
    pub fn from_config(t: ConfigTheme) -> Self {
        match t {
            ConfigTheme::Monochrome => Theme {
                accent: Color::White,
                dim: Color::DarkGray,
                changed: Color::White,
                error: Color::White,
                current: Color::White,
                monochrome: true,
            },
            ConfigTheme::Light => Theme {
                accent: Color::Blue,
                dim: Color::Gray,
                changed: Color::Red,
                error: Color::Red,
                current: Color::Green,
                monochrome: false,
            },
            ConfigTheme::Dark => Theme {
                accent: Color::Cyan,
                dim: Color::DarkGray,
                changed: Color::Yellow,
                error: Color::Red,
                current: Color::Green,
                monochrome: false,
            },
        }
    }

    pub fn title(&self, focused: bool) -> Style {
        let base = Style::default().add_modifier(Modifier::BOLD);
        if focused {
            base.fg(self.accent)
        } else {
            base.fg(self.dim)
        }
    }

    pub fn changed(&self) -> Style {
        let s = Style::default()
            .fg(self.changed)
            .add_modifier(Modifier::BOLD);
        if self.monochrome {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            s
        }
    }

    pub fn current_line(&self) -> Style {
        if self.monochrome {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .fg(self.current)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub fn error(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }
}

/// Glyphs used across the UI, with ASCII fallbacks so Bind degrades on
/// terminals/fonts without the Unicode versions.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub current: &'static str,
    pub breakpoint: &'static str,
    pub bp_current: &'static str,
    pub bullet: &'static str,
}

impl Glyphs {
    pub fn new(unicode: bool) -> Self {
        if unicode {
            Glyphs {
                current: "▶",
                breakpoint: "●",
                bp_current: "◆",
                bullet: "·",
            }
        } else {
            Glyphs {
                current: ">",
                breakpoint: "*",
                bp_current: "#",
                bullet: ".",
            }
        }
    }
}
