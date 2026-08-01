//! Transient overlays: whichkey hint, palette, help, confirms, pickers, images.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_image::StatefulImage;

use crate::actions::{ACTIONS, PaletteState, prefix_options};
use crate::state::{App, Overlay};
use crate::theme::Token;
use crate::ui::{THEME, panel};

pub fn draw_overlay(f: &mut Frame, app: &mut App) {
    let Some(overlay) = &mut app.overlay else {
        return;
    };
    match overlay {
        Overlay::Help { scroll } => draw_help(f, *scroll),
        Overlay::Palette(state) => draw_palette(f, state),
        Overlay::ConfirmDelete { ids } => draw_confirm(
            f,
            "delete sessions",
            &format!("Delete {} conversation(s)?", ids.len()),
            "This cannot be undone.",
        ),
        Overlay::ConfirmDeleteSource { name, .. } => draw_confirm(
            f,
            "delete source",
            &format!("Delete source '{name}' and all its conversations?"),
            "This cannot be undone.",
        ),
        Overlay::AgentPicker { agents, cursor, .. } => {
            draw_agent_picker(f, agents, *cursor, &app.config.resume.default_agent);
        }
        Overlay::ImageViewer(viewer) => {
            let count = app.chat_images.len();
            let label = app
                .chat_images
                .get(viewer.index)
                .map(|e| e.label.clone())
                .unwrap_or_default();
            let area = centered_rect(90, 88, f.area());
            f.render_widget(Clear, area);
            let block = panel(
                &format!("image {}/{count} · {label}", viewer.index + 1),
                true,
            );
            let inner = block.inner(area);
            f.render_widget(block, area);
            if let Some(protocol) = viewer.protocol.as_mut() {
                f.render_stateful_widget(StatefulImage::default(), inner, protocol);
            } else {
                let msg = viewer.error.clone().unwrap_or_else(|| "Loading…".into());
                f.render_widget(
                    Paragraph::new(msg)
                        .style(THEME.fg(Token::Muted))
                        .alignment(Alignment::Center),
                    inner,
                );
            }
            let hint = Paragraph::new(Line::from(Span::styled(
                " n:next  p:previous  Esc:close ",
                THEME.fg(Token::Muted),
            )))
            .alignment(Alignment::Right);
            let hint_area = Rect {
                y: area.bottom().saturating_sub(1),
                height: 1,
                ..area
            };
            f.render_widget(hint, hint_area);
        }
    }
}

pub fn draw_whichkey(f: &mut Frame, prefix: char, status_area: Rect) {
    let options = prefix_options(prefix);
    if options.is_empty() {
        return;
    }
    let width = options
        .iter()
        .map(|(k, l)| k.len() + l.len() + 5)
        .max()
        .unwrap_or(20)
        .max(16) as u16
        + 4;
    let height = options.len() as u16 + 2;
    let area = Rect {
        x: f.area().width.saturating_sub(width + 1),
        y: status_area.y.saturating_sub(height),
        width: width.min(f.area().width),
        height,
    };
    f.render_widget(Clear, area);
    let block = panel(&format!("{prefix} …"), true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = options
        .iter()
        .map(|(k, label)| {
            Line::from(vec![
                Span::styled(format!("{k:>2} "), THEME.fg_bold(Token::Accent)),
                Span::styled(format!(" {label}"), THEME.fg(Token::Primary)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_palette(f: &mut Frame, state: &PaletteState) {
    let width = (f.area().width * 6 / 10).clamp(40, 72);
    let height = (state.matches.len() as u16 + 4).clamp(6, 18);
    let area = Rect {
        x: (f.area().width.saturating_sub(width)) / 2,
        y: 2,
        width,
        height,
    };
    f.render_widget(Clear, area);
    let block = panel("commands", true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let input = Line::from(vec![
        Span::styled("❯ ", THEME.fg_bold(Token::Accent)),
        Span::styled(state.query.clone(), THEME.fg(Token::Primary)),
        Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
    ]);
    f.render_widget(Paragraph::new(input), rows[0]);

    let label_w = rows[1].width.saturating_sub(10) as usize;
    let items: Vec<ListItem> = state
        .matches
        .iter()
        .map(|&i| {
            let action = &ACTIONS[i];
            let label = crate::markdown::truncate_str(action.label, label_w);
            let pad = label_w.saturating_sub(label.chars().count());
            ListItem::new(Line::from(vec![
                Span::styled(label, THEME.fg(Token::Primary)),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(action.keys, THEME.fg(Token::Muted)),
            ]))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(THEME.color(Token::Bar))
            .add_modifier(Modifier::BOLD),
    );
    let mut lstate = ListState::default().with_selected(Some(state.cursor));
    f.render_stateful_widget(list, rows[1], &mut lstate);
}

fn draw_agent_picker(f: &mut Frame, agents: &[String], cursor: usize, default_agent: &str) {
    let height = (agents.len() as u16 + 2).clamp(4, 14);
    let width = 40u16.min(f.area().width);
    let area = Rect {
        x: (f.area().width.saturating_sub(width)) / 2,
        y: (f.area().height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, area);
    let block = panel("resume with", true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = agents
        .iter()
        .map(|a| {
            let default_marker = if a == default_agent {
                "  (default)"
            } else {
                ""
            };
            ListItem::new(Line::from(vec![
                Span::styled(a.clone(), THEME.fg(Token::Primary)),
                Span::styled(default_marker, THEME.fg(Token::Muted)),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(
        Style::default()
            .bg(THEME.color(Token::Bar))
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected(Some(cursor));
    f.render_stateful_widget(list, inner, &mut state);
}

fn draw_confirm(f: &mut Frame, title: &str, question: &str, warning: &str) {
    let area = centered_rect(50, 24, f.area());
    f.render_widget(Clear, area);
    let mut block = panel(title, true);
    block = block.border_style(THEME.fg(Token::Danger));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            question.to_string(),
            THEME.fg_bold(Token::Primary),
        )),
        Line::from(Span::styled(warning.to_string(), THEME.fg(Token::Muted))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", THEME.on_bar_bold(Token::Danger)),
            Span::raw(" yes   "),
            Span::styled(" n ", THEME.on_bar_bold(Token::Success)),
            Span::raw(" no"),
        ]),
    ];
    f.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        inner,
    );
}

fn draw_help(f: &mut Frame, scroll: usize) {
    let area = centered_rect(64, 84, f.area());
    f.render_widget(Clear, area);
    let block = panel("help", true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let section = |t: &str| Line::from(Span::styled(t.to_string(), THEME.fg_bold(Token::Accent)));
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(format!("  {k:<12}"), THEME.fg_bold(Token::Primary)),
            Span::styled(d.to_string(), THEME.fg(Token::Muted)),
        ])
    };

    let help: Vec<Line> = vec![
        section("navigate"),
        key("j/k ↑/↓", "move"),
        key("gg / G", "top / bottom"),
        key("Ctrl-d/u", "half page"),
        key("h/l ←/→", "collapse group · focus chat / sidebar"),
        key("Enter", "open session / toggle group"),
        key("Tab", "hide or show the sidebar"),
        Line::from(""),
        section("organize"),
        key(
            "z a/r/A/d/f",
            "group by agent / repo / agent▸repo / month / flat",
        ),
        key("z c / z o", "collapse / expand all groups"),
        key("s d/D/t", "sort newest / oldest / title"),
        key("f", "filter list (F clears)"),
        Line::from(""),
        section("search"),
        key("/", "full-text search (Tab cycles local/all/remote)"),
        key("x", "clear search results"),
        Line::from(""),
        section("act"),
        key(": Ctrl-P", "command palette"),
        key("Ctrl-R", "resume in default agent"),
        key("R", "resume with agent picker"),
        key("i", "view images (kitty/sixel graphics)"),
        key("Space", "mark session (on a group: mark all)"),
        key("Ctrl-A / V", "mark all / clear marks"),
        key("d", "delete marked or current (on agent group: source)"),
        key("r", "refresh"),
        key("q", "quit"),
    ];

    let max_scroll = help.len().saturating_sub(inner.height as usize);
    let scroll = scroll.min(max_scroll);
    f.render_widget(
        Paragraph::new(help).scroll((u16::try_from(scroll).unwrap_or(0), 0)),
        inner,
    );
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
