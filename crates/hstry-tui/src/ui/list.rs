//! The session list: two-line rows (title, then agent · repo · age · size),
//! grouped headers, and content-search hits grouped per session.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::markdown::{highlight_lines, truncate_str};
use crate::state::{App, Focus, Row, Scope, excerpt, relative_time, repo_name};
use crate::theme::{Token, adapter_color};
use crate::ui::{THEME, panel};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::List;
    let title = if app.showing_content() {
        format!("{} sessions match", app.content_results.len())
    } else if app.query.trim().is_empty() {
        "sessions".to_string()
    } else {
        format!("{} of {} sessions", app.visible.len(), app.sessions.len())
    };
    let block = panel(&title, active);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.rows.is_empty() {
        let hint = if app.sessions.is_empty() {
            "No sessions yet.\n\nRun `hstry sync` to import history from Claude Code, Codex, Pi and more."
                .to_string()
        } else if app.scope == Scope::Content && (app.searching || app.query_changed_at.is_some()) {
            "Searching…".to_string()
        } else {
            format!(
                "Nothing matches “{}”.\n\nTab switches between session and full-text search; Esc clears.",
                app.query.trim()
            )
        };
        f.render_widget(
            Paragraph::new(hint)
                .style(THEME.fg(Token::Muted))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    // One column is the selection gutter.
    let width = (inner.width as usize).saturating_sub(1);
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| render_row(app, row, width))
        .collect();

    // Background only, so titles keep their weight and matches their accent.
    let highlight = THEME.bg(Token::Bar);
    let list = List::new(items)
        .highlight_style(highlight)
        .highlight_symbol(if active { "▌" } else { "▏" })
        .repeat_highlight_symbol(true)
        .scroll_padding(1);
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
            let visible = &app.visible[*vis];
            let session = &app.sessions[visible.idx];
            let indent = "  ".repeat(*depth as usize);
            let marked = app.marked.contains(&session.conv.id);
            let lead = if marked {
                Span::styled("✓ ", THEME.fg_bold(Token::Warn))
            } else {
                Span::raw("  ")
            };

            let title_room = width.saturating_sub(2 + indent.len()).max(8);
            let mut title_line = vec![lead, Span::raw(indent.clone())];
            title_line.extend(highlighted_title(
                session.title(),
                &visible.title_positions,
                title_room,
            ));

            let meta = meta_spans(
                &session.adapter,
                session.conv.workspace.as_deref(),
                &relative_time(session.last_active()),
                Some(session.message_count),
                width.saturating_sub(4 + indent.len()),
            );
            let mut meta_line = vec![Span::raw(format!("  {indent}  "))];
            meta_line.extend(meta);

            ListItem::new(vec![Line::from(title_line), Line::from(meta_line)])
        }
        Row::Hits { idx } => {
            let group = &app.content_results[*idx];
            let best = group.best();
            let session = app.session_by_id(group.conversation_id);
            let title = session.map_or_else(
                || best.title.clone().unwrap_or_else(|| "(untitled)".into()),
                |s| s.title().to_string(),
            );
            let count = group.hits.len();
            let count_label = if count == 1 {
                "1 hit".to_string()
            } else {
                format!("{count} hits")
            };
            let room = width.saturating_sub(4 + count_label.len()).max(8);
            let title = truncate_str(&title, room);
            let pad = room.saturating_sub(title.width());
            let title_line = Line::from(vec![
                Span::raw("  "),
                Span::styled(title, THEME.fg_bold(Token::Primary)),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(count_label, THEME.fg(Token::Accent)),
            ]);

            let adapter = session.map_or(best.source_adapter.as_str(), |s| s.adapter.as_str());
            let mut meta = meta_spans(
                adapter,
                best.workspace.as_deref(),
                &relative_time(best.conv_updated_at.unwrap_or(best.conv_created_at)),
                None,
                width.saturating_sub(4),
            );
            if let Some(host) = &best.host {
                meta.push(Span::styled(format!(" · @{host}"), THEME.fg(Token::Warn)));
            }
            let mut meta_line = vec![Span::raw("    ")];
            meta_line.extend(meta);

            let query = app.content_query.clone().unwrap_or_default();
            let snippet = excerpt(&best.content, &query, width.saturating_sub(6));
            let snippet_line = Line::from(vec![
                Span::raw("    "),
                Span::styled("“", THEME.fg(Token::Muted)),
                Span::styled(
                    snippet,
                    THEME.fg(Token::Muted).add_modifier(Modifier::ITALIC),
                ),
                Span::styled("”", THEME.fg(Token::Muted)),
            ]);
            let snippet_line = highlight_lines(vec![snippet_line], &query)
                .pop()
                .unwrap_or_default();

            ListItem::new(vec![title_line, Line::from(meta_line), snippet_line])
        }
    }
}

/// `● agent · repo · age · N msgs`, trimmed to `room` columns.
fn meta_spans(
    adapter: &str,
    workspace: Option<&str>,
    age: &str,
    messages: Option<i64>,
    room: usize,
) -> Vec<Span<'static>> {
    let mut parts: Vec<String> = vec![adapter.to_string()];
    if let Some(ws) = workspace {
        parts.push(repo_name(ws));
    }
    parts.push(age.to_string());
    if let Some(n) = messages.filter(|n| *n > 0) {
        parts.push(if n == 1 {
            "1 msg".into()
        } else {
            format!("{n} msgs")
        });
    }
    let text = truncate_str(&parts.join(" · "), room.saturating_sub(2).max(4));
    vec![
        Span::styled("● ", Style::default().fg(adapter_color(adapter))),
        Span::styled(text, THEME.fg(Token::Muted)),
    ]
}

/// Title spans with fuzzy-matched characters emphasized.
fn highlighted_title(title: &str, positions: &[usize], room: usize) -> Vec<Span<'static>> {
    let title = truncate_str(title.trim(), room);
    if positions.is_empty() {
        return vec![Span::styled(title, THEME.fg_bold(Token::Primary))];
    }
    let normal = THEME.fg_bold(Token::Primary);
    let matched = THEME
        .fg_bold(Token::Accent)
        .add_modifier(Modifier::UNDERLINED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_matched = false;
    for (i, ch) in title.chars().enumerate() {
        let is_match = positions.binary_search(&i).is_ok();
        if is_match != run_matched && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_matched { matched } else { normal },
            ));
        }
        run_matched = is_match;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(
            run,
            if run_matched { matched } else { normal },
        ));
    }
    spans
}
