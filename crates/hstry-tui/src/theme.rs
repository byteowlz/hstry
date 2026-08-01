//! ANSI-token theme: two surface shades, one accent, semantic state colors.

use ratatui::style::{Color, Modifier, Style};

/// Semantic color slots. Code references tokens, never raw colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// Primary text.
    Primary,
    /// Secondary text, thin borders.
    Muted,
    /// The single accent: focus + primary actions.
    Accent,
    /// Dark fill for header/status bars and panel chrome.
    Bar,
    /// Success / confirmation.
    Success,
    /// Destructive / error.
    Danger,
    /// Warning / attention.
    Warn,
}

/// Maps tokens to named ANSI colors so the terminal owns rendering.
#[derive(Debug, Clone, Copy)]
pub struct Theme;

impl Theme {
    pub const fn color(self, token: Token) -> Color {
        match token {
            Token::Primary => Color::Reset,
            Token::Muted => Color::DarkGray,
            Token::Accent => Color::Blue,
            Token::Bar => Color::Black,
            Token::Success => Color::Green,
            Token::Danger => Color::Red,
            Token::Warn => Color::Yellow,
        }
    }

    pub fn fg(self, token: Token) -> Style {
        Style::default().fg(self.color(token))
    }

    pub fn fg_bold(self, token: Token) -> Style {
        self.fg(token).add_modifier(Modifier::BOLD)
    }

    pub fn bg(self, token: Token) -> Style {
        Style::default().bg(self.color(token))
    }

    /// Text drawn on a `Bar`-filled strip.
    pub fn on_bar(self, token: Token) -> Style {
        Style::default()
            .fg(self.color(token))
            .bg(self.color(Token::Bar))
    }

    pub fn on_bar_bold(self, token: Token) -> Style {
        self.on_bar(token).add_modifier(Modifier::BOLD)
    }
}

/// Stable origin color for a source adapter (dot + label in the sidebar).
pub fn adapter_color(adapter: &str) -> Color {
    let key = adapter.to_lowercase();
    match key.as_str() {
        a if a.contains("claude") => Color::Yellow,
        a if a.contains("pi") => Color::Magenta,
        a if a.contains("codex") => Color::Cyan,
        a if a.contains("opencode") => Color::Green,
        a if a.contains("goose") => Color::LightBlue,
        a if a.contains("chatgpt") || a.contains("openai") => Color::LightGreen,
        a if a.contains("gemini") => Color::LightMagenta,
        a if a.contains("octo") => Color::LightCyan,
        _ => {
            const PALETTE: [Color; 6] = [
                Color::LightYellow,
                Color::LightRed,
                Color::Cyan,
                Color::Magenta,
                Color::Green,
                Color::LightBlue,
            ];
            let hash: usize = key.bytes().fold(0usize, |acc, b| {
                acc.wrapping_mul(31).wrapping_add(b as usize)
            });
            PALETTE[hash % PALETTE.len()]
        }
    }
}
