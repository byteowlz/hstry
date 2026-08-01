//! The chat reader: pretty message cards over the markdown renderer.

use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use hstry_core::models::{Message, MessageRole};

use crate::images::{ImageEntry, extract_images};
use crate::markdown::{render_markdown, truncate_str};
use crate::state::{App, Focus};
use crate::theme::Token;
use crate::ui::{THEME, panel};

const fn role_color(role: &MessageRole) -> Color {
    match role {
        MessageRole::User => Color::Green,
        MessageRole::Assistant => Color::Blue,
        MessageRole::System => Color::Yellow,
        MessageRole::Tool => Color::Magenta,
        MessageRole::Other => Color::Gray,
    }
}

const fn role_glyph(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "❯",
        MessageRole::Assistant => "✦",
        MessageRole::System => "⚙",
        MessageRole::Tool => "⚒",
        MessageRole::Other => "·",
    }
}

const fn role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "you",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
        MessageRole::Other => "other",
    }
}

/// Build the full transcript as styled lines plus any images found in parts.
pub fn build_chat_lines(
    messages: &[Message],
    highlight: Option<&str>,
) -> (Vec<Line<'static>>, Vec<ImageEntry>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<ImageEntry> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }

        let color = role_color(&msg.role);
        let mut header = vec![Span::styled(
            format!("{} {}", role_glyph(&msg.role), role_name(&msg.role)),
            Style::default().fg(color).bold(),
        )];
        if let Some(ts) = msg.created_at {
            let local: DateTime<Local> = ts.into();
            header.push(Span::styled(
                format!("  {}", local.format("%H:%M")),
                THEME.fg(Token::Muted),
            ));
        }
        if let Some(model) = &msg.model {
            header.push(Span::styled(format!("  {model}"), THEME.fg(Token::Muted)));
        }
        lines.push(Line::from(header));

        for line in render_markdown(&msg.content, &msg.role, highlight) {
            let mut spans = vec![Span::styled("▏ ", Style::default().fg(color))];
            spans.extend(line.spans);
            lines.push(Line { spans, ..line });
        }

        for entry in extract_images(&msg.parts_json) {
            images.push(entry.clone());
            lines.push(Line::from(vec![
                Span::styled("▏ ", Style::default().fg(color)),
                Span::styled(
                    format!("▨ image {}: {}  (press i)", images.len(), entry.label),
                    Style::default().fg(Color::Cyan),
                ),
            ]));
        }
    }

    (lines, images)
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let active = app.focus == Focus::Chat;

    let title = app.cursor_conversation().map_or_else(
        || "chat".to_string(),
        |c| {
            truncate_str(
                c.title.as_deref().unwrap_or("(untitled)"),
                area.width.saturating_sub(6) as usize,
            )
        },
    );

    let mut block = panel(&title, active);
    if let Some(conv) = app.cursor_conversation() {
        let mut meta = format!("{} msgs", conv.message_count.max(app.messages.len() as i64));
        if let Some(model) = &conv.model {
            meta = format!("{meta} · {model}");
        }
        if let Some(ws) = &conv.workspace {
            meta = format!("{meta} · {}", crate::state::shorten_workspace(ws));
        }
        if !app.chat_images.is_empty() {
            meta = format!("{meta} · {} images", app.chat_images.len());
        }
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {meta} "),
            THEME.fg(Token::Muted),
        )));
    }

    let inner = block.inner(area);
    f.render_widget(block, area);
    app.chat_height = inner.height;

    if app.chat_lines.is_empty() {
        let hint = if app.rows.is_empty() {
            "Nothing here yet — run `hstry sync` to import sessions."
        } else {
            "Select a session — Enter loads it, l focuses the chat."
        };
        let empty = Paragraph::new(hint)
            .style(THEME.fg(Token::Muted))
            .wrap(Wrap { trim: true });
        f.render_widget(empty, inner);
        return;
    }

    let max_scroll = app.chat_lines.len().saturating_sub(1);
    app.chat_scroll = app.chat_scroll.min(max_scroll);
    let scroll = u16::try_from(app.chat_scroll).unwrap_or(u16::MAX);

    let paragraph = Paragraph::new(app.chat_lines.clone())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(paragraph, inner);
}
