//! Top-level layout: query bar, list + transcript, action bar, overlays.

pub mod chat;
pub mod list;
pub mod overlays;
#[cfg(test)]
mod render_tests;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::state::{App, Focus, Mode, NARROW_WIDTH, Scope};
use crate::theme::{Theme, Token};

pub const THEME: Theme = Theme;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_query_bar(f, app, chunks[0]);

    app.narrow = area.width < NARROW_WIDTH;
    if app.narrow {
        // One pane at a time: the list, or the session being read.
        if app.focus == Focus::Transcript {
            app.list_area = Rect::default();
            app.chat_area = chunks[1];
            chat::draw(f, app, chunks[1]);
        } else {
            app.list_area = chunks[1];
            app.chat_area = Rect::default();
            list::draw(f, app, chunks[1]);
        }
    } else {
        let list_w = (u32::from(chunks[1].width) * 38 / 100).clamp(40, 64) as u16;
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(list_w),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(chunks[1]);
        app.list_area = body[0];
        app.chat_area = body[2];
        list::draw(f, app, body[0]);
        chat::draw(f, app, body[2]);
    }

    draw_action_bar(f, app, chunks[2]);

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

/// ` hstry  ⌕ query▏            sessions│content   12 of 150 `
fn draw_query_bar(f: &mut Frame, app: &App, area: Rect) {
    let typing = app.mode == Mode::Query;
    let brand = Span::styled(" hstry ", THEME.on_bar_bold(Token::Accent));

    // Right side: scope switch and counts.
    let scope_span = |scope: Scope| {
        if app.scope == scope {
            Span::styled(
                format!(" {} ", scope.label()),
                THEME
                    .fg_bold(Token::Accent)
                    .bg(THEME.color(Token::Bar))
                    .add_modifier(Modifier::REVERSED),
            )
        } else {
            Span::styled(format!(" {} ", scope.label()), THEME.on_bar(Token::Muted))
        }
    };
    let count = if app.showing_content() {
        let hits: usize = app.content_results.iter().map(|g| g.hits.len()).sum();
        format!("{hits} hits ")
    } else if app.query.trim().is_empty() || app.scope == Scope::Content {
        format!("{} sessions ", app.sessions.len())
    } else {
        format!("{} of {} ", app.visible.len(), app.sessions.len())
    };
    let mut right = vec![
        scope_span(Scope::Sessions),
        scope_span(Scope::Content),
        Span::styled("  ", THEME.bg(Token::Bar)),
        Span::styled(count, THEME.on_bar(Token::Muted)),
    ];
    if !app.marked.is_empty() {
        right.insert(
            0,
            Span::styled(
                format!("{} marked  ", app.marked.len()),
                THEME.on_bar_bold(Token::Warn),
            ),
        );
    }
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();

    // The input field is drawn on the terminal background so it reads as an
    // inset field inside the bar.
    let field_w = (area.width as usize)
        .saturating_sub(brand.content.width() + right_w + 3)
        .max(10);
    let mut field: Vec<Span<'static>> = vec![Span::styled(
        " ⌕ ",
        if typing {
            THEME.fg_bold(Token::Accent)
        } else {
            THEME.fg(Token::Muted)
        },
    )];
    let text_room = field_w.saturating_sub(4);
    if app.query.is_empty() && !typing {
        field.push(Span::styled(
            crate::markdown::truncate_str("press / to find a session", text_room),
            THEME.fg(Token::Muted).add_modifier(Modifier::ITALIC),
        ));
    } else {
        let chars: Vec<char> = app.query.chars().collect();
        // Keep the cursor visible when the query is longer than the field.
        let start = app.query_cursor.saturating_sub(text_room.saturating_sub(1));
        let visible: Vec<char> = chars.iter().skip(start).take(text_room).copied().collect();
        let cur = app.query_cursor - start;
        let before: String = visible.iter().take(cur).collect();
        let at: String = visible.get(cur).map_or(" ".into(), |c| c.to_string());
        let after: String = visible.iter().skip(cur + 1).collect();
        field.push(Span::styled(before, THEME.fg_bold(Token::Primary)));
        if typing {
            field.push(Span::styled(
                at,
                THEME.fg(Token::Primary).add_modifier(Modifier::REVERSED),
            ));
        } else {
            field.push(Span::styled(at, THEME.fg_bold(Token::Primary)));
        }
        field.push(Span::styled(after, THEME.fg_bold(Token::Primary)));
        if app.searching {
            field.push(Span::styled("  searching…", THEME.fg(Token::Muted)));
        }
    }
    let used: usize = field.iter().map(|s| s.content.width()).sum();
    field.push(Span::raw(" ".repeat(field_w.saturating_sub(used))));

    let mut spans = vec![brand, Span::styled(" ", THEME.bg(Token::Bar))];
    spans.extend(field);
    let left_w: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::styled(
        " ".repeat((area.width as usize).saturating_sub(left_w + right_w)),
        THEME.bg(Token::Bar),
    ));
    spans.extend(right);

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(THEME.bg(Token::Bar)),
        area,
    );
}

/// Context actions for what is selected, or a fading status message.
fn draw_action_bar(f: &mut Frame, app: &App, area: Rect) {
    let status = app
        .status
        .as_ref()
        .filter(|(_, at)| at.elapsed().as_secs() < 4)
        .map(|(m, _)| m.clone());

    let actions: &[(&str, &str)] = match (app.mode, app.focus) {
        (Mode::Query, _) => &[
            ("type", "to find"),
            ("↑↓", "select"),
            ("Tab", "sessions/content"),
            ("⏎", "done"),
            ("Esc", "clear"),
        ],
        (Mode::Normal, Focus::Transcript) => &[
            ("o", "open in…"),
            ("y", "copy"),
            (
                "t",
                if app.expand_details {
                    "collapse"
                } else {
                    "expand"
                },
            ),
            ("j/k", "scroll"),
            ("e", "export"),
            ("Esc", "back"),
            ("?", "help"),
        ],
        (Mode::Normal, Focus::List) if app.cursor_conversation_id().is_some() => &[
            ("⏎", "read"),
            ("o", "open in…"),
            ("y", "copy"),
            ("Y", "copy id"),
            ("p", "copy path"),
            ("e", "export"),
            ("d", "delete"),
            ("/", "find"),
            ("?", "help"),
        ],
        (Mode::Normal, Focus::List) => &[
            ("⏎", "expand/collapse"),
            ("/", "find"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };

    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", THEME.bg(Token::Bar))];
    if let Some(msg) = status {
        spans.push(Span::styled(
            format!("{msg}  "),
            THEME.on_bar_bold(Token::Success),
        ));
    }
    let budget = area.width as usize;
    for (key, label) in actions {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let piece_w = key.width() + label.width() + 4;
        if used + piece_w > budget {
            break;
        }
        spans.push(Span::styled(
            (*key).to_string(),
            THEME.on_bar_bold(Token::Accent),
        ));
        spans.push(Span::styled(
            format!(" {label}   "),
            THEME.on_bar(Token::Muted),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(THEME.bg(Token::Bar)),
        area,
    );
}
