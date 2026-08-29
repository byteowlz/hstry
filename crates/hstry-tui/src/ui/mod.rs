//! Top-level layout: header bar, sidebar, chat, status bar, overlays.

pub mod chat;
pub mod overlays;
pub mod sidebar;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};

use crate::state::{App, Focus, Mode};
use crate::theme::{Theme, Token};

pub const THEME: Theme = Theme;

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);

    if app.sidebar_visible {
        let sidebar_w = (u32::from(chunks[1].width) * 34 / 100).clamp(28, 46) as u16;
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_w), Constraint::Min(0)])
            .split(chunks[1]);
        app.sidebar_area = body[0];
        app.chat_area = body[1];
        sidebar::draw(f, app, body[0]);
        chat::draw(f, app, body[1]);
    } else {
        app.sidebar_area = Rect::default();
        app.chat_area = chunks[1];
        chat::draw(f, app, chunks[1]);
    }

    draw_status(f, app, chunks[2]);

    if let Some(prefix) = app.pending_prefix {
        overlays::draw_whichkey(f, prefix, chunks[2]);
    }

    overlays::draw_overlay(f, app);
}

/// A titled rounded panel; active = accent border.
pub fn panel(title: &str, active: bool) -> Block<'static> {
    let border = if active {
        THEME.fg(Token::Accent)
    } else {
        THEME.fg(Token::Muted)
    };
    Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {title} "),
            if active {
                THEME.fg_bold(Token::Accent)
            } else {
                THEME.fg_bold(Token::Muted)
            },
        )]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1))
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" hstry ", THEME.on_bar_bold(Token::Accent)),
        Span::styled(" ", THEME.bg(Token::Bar)),
    ];

    if app.show_search_results {
        spans.push(Span::styled(
            format!(
                "search “{}” · {} hits",
                app.last_search_query.as_deref().unwrap_or(""),
                app.search_results.len()
            ),
            THEME.on_bar(Token::Warn),
        ));
    } else {
        spans.push(Span::styled(
            format!("group:{}", app.group_by.label()),
            THEME.on_bar(Token::Primary),
        ));
        spans.push(Span::styled("  ", THEME.bg(Token::Bar)));
        spans.push(Span::styled(
            format!("sort:{}", app.sort_order.label()),
            THEME.on_bar(Token::Muted),
        ));
        if !app.list_filter.is_empty() {
            spans.push(Span::styled("  ", THEME.bg(Token::Bar)));
            spans.push(Span::styled(
                format!("filter:“{}”", app.list_filter),
                THEME.on_bar(Token::Warn),
            ));
        }
        spans.push(Span::styled("  ", THEME.bg(Token::Bar)));
        spans.push(Span::styled(
            format!("{} sessions", app.visible.len()),
            THEME.on_bar(Token::Muted),
        ));
    }

    if !app.marked.is_empty() {
        spans.push(Span::styled("  ", THEME.bg(Token::Bar)));
        spans.push(Span::styled(
            format!("{} marked", app.marked.len()),
            THEME.on_bar_bold(Token::Warn),
        ));
    }

    let bar = Paragraph::new(Line::from(spans)).style(THEME.bg(Token::Bar));
    f.render_widget(bar, area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let line = match &app.mode {
        Mode::Search { query, cursor } => input_line(
            "search",
            query,
            *cursor,
            &format!("scope:{} (Tab)", app.search_scope.label()),
        ),
        Mode::Filter { query, cursor } => input_line("filter", query, *cursor, "Enter to apply"),
        Mode::Normal => {
            let mut spans = Vec::new();
            let focus_label = match app.focus {
                Focus::Sidebar => " SESSIONS ",
                Focus::Chat => " CHAT ",
            };
            spans.push(Span::styled(focus_label, THEME.on_bar_bold(Token::Accent)));
            spans.push(Span::styled(" ", THEME.bg(Token::Bar)));

            let msg = app
                .status
                .as_ref()
                .filter(|(_, at)| at.elapsed().as_secs() < 5)
                .map(|(m, _)| m.as_str())
                .unwrap_or("");
            if msg.is_empty() {
                spans.push(Span::styled(
                    "?:help  ::palette  /:search  f:filter  Tab:sidebar  R:resume",
                    THEME.on_bar(Token::Muted),
                ));
            } else {
                spans.push(Span::styled(msg.to_string(), THEME.on_bar(Token::Primary)));
            }
            Line::from(spans)
        }
    };

    let bar = Paragraph::new(line).style(THEME.bg(Token::Bar));
    f.render_widget(bar, area);
}

fn input_line(label: &str, query: &str, cursor: usize, hint: &str) -> Line<'static> {
    let before: String = query.chars().take(cursor).collect();
    let at: String = query
        .chars()
        .nth(cursor)
        .map_or(" ".to_string(), |c| c.to_string());
    let after: String = query.chars().skip(cursor + 1).collect();

    Line::from(vec![
        Span::styled(format!(" {label} "), THEME.on_bar_bold(Token::Warn)),
        Span::styled(" ", THEME.bg(Token::Bar)),
        Span::styled(before, THEME.on_bar(Token::Primary)),
        Span::styled(
            at,
            THEME
                .on_bar(Token::Primary)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::styled(after, THEME.on_bar(Token::Primary)),
        Span::styled(format!("   {hint}"), THEME.on_bar(Token::Muted)),
    ])
}
