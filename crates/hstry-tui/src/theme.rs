//! ANSI-token theme: two surface shades, one accent, semantic state colors.

use std::sync::atomic::{AtomicBool, Ordering};

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

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Record whether the terminal has a light background. Surfaces and muted
/// text are the only tokens that depend on it: everything else is a named
/// ANSI color the terminal theme already adapts.
pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Detect the terminal background: `HSTRY_THEME=light|dark` wins, then an
/// OSC 11 query, then `COLORFGBG`. Unknown means dark.
pub fn detect_light() -> bool {
    match std::env::var("HSTRY_THEME").ok().as_deref() {
        Some("light") => return true,
        Some("dark") => return false,
        _ => {}
    }
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = std::time::Duration::from_millis(400);
    if let Ok(mode) = terminal_colorsaurus::theme_mode(options) {
        return mode == terminal_colorsaurus::ThemeMode::Light;
    }
    // COLORFGBG="fg;bg": ANSI background 7 or 15 means a light terminal.
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| v.rsplit(';').next().and_then(|bg| bg.parse::<u8>().ok()))
        .is_some_and(|bg| bg == 7 || bg == 15)
}

/// Maps tokens to named ANSI colors so the terminal owns rendering.
#[derive(Debug, Clone, Copy)]
pub struct Theme;

impl Theme {
    pub fn color(self, token: Token) -> Color {
        let light = is_light();
        match token {
            Token::Primary => Color::Reset,
            Token::Muted if light => Color::DarkGray,
            Token::Muted => Color::Gray,
            Token::Accent => Color::Blue,
            // The surface one step away from the background: ANSI black on
            // dark terminals, ANSI white (light gray) on light ones.
            Token::Bar if light => Color::White,
            Token::Bar => Color::Black,
            Token::Success => Color::Green,
            Token::Danger => Color::Red,
            Token::Warn if light => Color::Rgb(0xb5, 0x76, 0x00),
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
