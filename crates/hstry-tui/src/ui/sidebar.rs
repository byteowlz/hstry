//! The session list: grouped, collapsible, origin-colored.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

use crate::markdown::truncate_str;
use crate::state::{App, Focus, GroupBy, Row, relative_time};
use crate::theme::{Token, adapter_color};
use crate::ui::{THEME, panel};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Sidebar;
    let title = if app.show_search_results {
        format!("results · {}", app.search_results.len())
    } else {
        "sessions".to_string()
    };
    let block = panel(&title, active);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.rows.is_empty() {
        let hint = if app.show_search_results {
            "No results — Esc to go back"
        } else if app.list_filter.is_empty() {
            "No sessions — run `hstry sync`"
        } else {
            "No matches — press F to clear the filter"
        };
        let empty = Paragraph::new(hint)
            .style(THEME.fg(Token::Muted))
            .wrap(Wrap { trim: true });
        f.render_widget(empty, inner);
        return;
    }

    let width = inner.width as usize;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| render_row(app, row, width))
        .collect();

    let highlight = if active {
        Style::default()
            .bg(THEME.color(Token::Bar))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(THEME.color(Token::Bar))
    };

    let list = List::new(items)
        .highlight_style(highlight)
        .scroll_padding(2);
    let mut state = ListState::default().with_selected(Some(app.cursor));
    f.render_stateful_widget(list, inner, &mut state);
}

fn render_row(app: &App, row: &Row, width: usize) -> ListItem<'static> {
    match row {
        Row::Header {
            label,
            count,
            depth,
            collapsed,
            color,
            ..
        } => {
            let indent = "  ".repeat(*depth as usize);
            let arrow = if *collapsed { "▸" } else { "▾" };
            let mut spans = vec![Span::styled(
                format!("{indent}{arrow} "),
                THEME.fg(Token::Muted),
            )];
            if let Some(c) = color {
                spans.push(Span::styled("● ", Style::default().fg(*c)));
            }
            spans.push(Span::styled(label.clone(), THEME.fg_bold(Token::Primary)));
            spans.push(Span::styled(format!("  {count}"), THEME.fg(Token::Muted)));
            ListItem::new(Line::from(spans))
        }
        Row::Session { vis, depth } => {
            let conv = &app.all_conversations[app.visible[*vis]];
            let indent = "  ".repeat(*depth as usize);
            let marked = app.marked.contains(&conv.id);
            let adapter = app.adapter_of(&conv.source_id);

            let mark_span = if marked {
                Span::styled("▎", THEME.fg_bold(Token::Warn))
            } else {
                Span::raw(" ")
            };
            let dot = Span::styled("● ", Style::default().fg(adapter_color(adapter)));

            let time = relative_time(conv.updated_at.unwrap_or(conv.created_at));
            let show_repo = matches!(app.group_by, GroupBy::Flat | GroupBy::Agent | GroupBy::Date);
            let mut meta = time;
            if show_repo && let Some(ws) = conv.workspace.as_deref() {
                meta = format!("{meta} · {}", crate::state::shorten_workspace(ws));
            }

            let overhead = indent.len() + 3 + meta.chars().count() + 2;
            let title_w = width.saturating_sub(overhead).max(8);
            let title = truncate_str(
                conv.title.as_deref().unwrap_or("(untitled)").trim(),
                title_w,
            );

            ListItem::new(Line::from(vec![
                mark_span,
                Span::raw(indent),
                dot,
                Span::styled(title, THEME.fg(Token::Primary)),
                Span::styled(format!("  {meta}"), THEME.fg(Token::Muted)),
            ]))
        }
        Row::Hit { idx } => {
            let hit = &app.search_results[*idx];
            let dot = Span::styled(
                "● ",
                Style::default().fg(adapter_color(&hit.source_adapter)),
            );
            let host = hit.host.as_deref();
            let title = truncate_str(
                hit.title.as_deref().unwrap_or("(untitled)"),
                width
                    .saturating_sub(host.map_or(0, |h| h.len() + 3) + 4)
                    .max(8),
            );
            let mut spans = vec![
                Span::raw(" "),
                dot,
                Span::styled(title, THEME.fg(Token::Primary)),
            ];
            if let Some(h) = host {
                spans.push(Span::styled(format!(" @{h}"), THEME.fg(Token::Warn)));
            }
            let snippet = truncate_str(&hit.snippet.replace('\n', " "), width.saturating_sub(6));
            ListItem::new(vec![
                Line::from(spans),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(snippet, THEME.fg(Token::Muted)),
                ]),
            ])
        }
    }
}
