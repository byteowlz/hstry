//! The chat reader: message cards over the markdown renderer.

use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use hstry_core::models::{Message, MessageRole};

use crate::images::{ImageEntry, extract_images};
use crate::markdown::{render_markdown, truncate_str};
use crate::state::{App, Focus};
use crate::theme::Token;
use crate::ui::{THEME, panel};

/// Width of the role gutter drawn left of every content line.
const GUTTER: usize = 2;

const fn role_color(role: &MessageRole) -> Color {
    match role {
        MessageRole::User => Color::Green,
        MessageRole::Assistant => Color::Blue,
        MessageRole::System => Color::Yellow,
        MessageRole::Tool => Color::Magenta,
        MessageRole::Other => Color::Gray,
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
///
/// `width` is the content width available for wrapping. `0` disables wrapping.
pub fn build_chat_lines(
    messages: &[Message],
    highlight: Option<&str>,
    width: usize,
) -> (Vec<Line<'static>>, Vec<ImageEntry>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<ImageEntry> = Vec::new();
    let mut last_model: Option<&str> = None;
    let mut prev_was_system = false;
    let body_width = width.saturating_sub(GUTTER);

    for msg in messages {
        let color = role_color(&msg.role);

        // System notices are one-liners: no header, no card, grouped together.
        if msg.role == MessageRole::System {
            if !lines.is_empty() && !prev_was_system {
                lines.push(Line::from(""));
            }
            let text = msg.content.trim().replace('\n', " ");
            let notice = Line::from(vec![
                Span::styled("· ", Style::default().fg(color)),
                Span::styled(text, THEME.fg(Token::Muted)),
            ]);
            lines.extend(wrap_line(notice, width, GUTTER));
            prev_was_system = true;
            continue;
        }
        prev_was_system = false;

        if !lines.is_empty() {
            lines.push(Line::from(""));
        }

        let mut header = vec![Span::styled(
            role_name(&msg.role),
            Style::default().fg(color).bold(),
        )];
        if let Some(ts) = msg.created_at {
            let local: DateTime<Local> = ts.into();
            header.push(Span::styled(
                format!("  {}", local.format("%H:%M")),
                THEME.fg(Token::Muted),
            ));
        }
        if let Some(model) = msg.model.as_deref()
            && last_model != Some(model)
        {
            header.push(Span::styled(format!("  {model}"), THEME.fg(Token::Muted)));
            last_model = Some(model);
        }
        lines.push(Line::from(header));

        let gutter = Span::styled("▏ ", Style::default().fg(color));
        for line in render_markdown(&msg.content, &msg.role, highlight) {
            for wrapped in wrap_line(line, body_width, 0) {
                let mut spans = vec![gutter.clone()];
                spans.extend(wrapped.spans);
                lines.push(Line { spans, ..wrapped });
            }
        }

        for entry in extract_images(&msg.parts_json) {
            images.push(entry.clone());
            lines.push(Line::from(vec![
                gutter.clone(),
                Span::styled(
                    format!("image {}: {}  (press i)", images.len(), entry.label),
                    Style::default().fg(Color::Cyan),
                ),
            ]));
        }
    }

    (lines, images)
}

/// Word-wrap a styled line to `width` columns, preserving span styles.
/// Continuation lines are indented by `indent` spaces. `width == 0` disables wrapping.
pub fn wrap_line(line: Line<'static>, width: usize, indent: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let style = line.style;
    let alignment = line.alignment;
    let cont_width = width.saturating_sub(indent).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0usize;
    let mut limit = width.max(1);

    let flush = |cur: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
        let first = out.is_empty();
        // Drop trailing whitespace-only spans so wrapped lines end cleanly.
        while cur
            .last()
            .is_some_and(|s| s.content.as_ref().trim().is_empty())
        {
            cur.pop();
        }
        if let Some(last) = cur.last_mut() {
            let trimmed = last.content.as_ref().trim_end().to_owned();
            last.content = trimmed.into();
        }
        let mut spans = Vec::with_capacity(cur.len() + 1);
        if !first && indent > 0 {
            spans.push(Span::raw(" ".repeat(indent)));
        }
        spans.append(cur);
        out.push(Line {
            spans,
            style,
            alignment,
        });
    };

    for span in line.spans {
        let sstyle = span.style;
        let text = span.content.as_ref().to_owned();
        // Split into words, keeping the whitespace that follows each word attached.
        let mut word = String::new();
        let mut pieces: Vec<String> = Vec::new();
        for ch in text.chars() {
            if ch.is_whitespace() {
                word.push(ch);
                if !word.trim().is_empty() {
                    pieces.push(std::mem::take(&mut word));
                } else if word.len() > 1 {
                    // Keep runs of whitespace as their own piece.
                    pieces.push(std::mem::take(&mut word));
                }
            } else {
                word.push(ch);
            }
        }
        if !word.is_empty() {
            pieces.push(word);
        }
        for piece in pieces {
            let pw = piece.width();
            let fit_w = piece.trim_end().width();
            if cur_w + fit_w <= limit {
                cur.push(Span::styled(piece, sstyle));
                cur_w += pw;
                continue;
            }
            if fit_w > limit {
                // Hard-break an overlong word.
                let mut chunk = String::new();
                let mut cw = 0;
                for ch in piece.chars() {
                    let w = ch.to_string().width();
                    if cur_w + cw + w > limit && (cur_w + cw) > 0 {
                        cur.push(Span::styled(std::mem::take(&mut chunk), sstyle));
                        flush(&mut cur, &mut out);
                        cur_w = 0;
                        cw = 0;
                        limit = cont_width;
                    }
                    chunk.push(ch);
                    cw += w;
                }
                if !chunk.is_empty() {
                    cur.push(Span::styled(chunk, sstyle));
                    cur_w += cw;
                }
                continue;
            }
            flush(&mut cur, &mut out);
            cur_w = 0;
            limit = cont_width;
            if piece.trim().is_empty() {
                continue;
            }
            cur.push(Span::styled(piece, sstyle));
            cur_w += pw;
        }
    }
    if !cur.is_empty() || out.is_empty() {
        flush(&mut cur, &mut out);
    }
    out
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
    if usize::from(inner.width) != app.chat_wrap_width {
        app.chat_wrap_width = usize::from(inner.width);
        app.rebuild_chat_lines();
    }

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

    let paragraph = Paragraph::new(app.chat_lines.clone()).scroll((scroll, 0));
    f.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wraps_at_word_boundaries_and_keeps_styles() {
        let bold = Style::default().bold();
        let line = Line::from(vec![
            Span::styled("hello big ", bold),
            Span::raw("wide world here"),
        ]);
        let out = wrap_line(line, 10, 0);
        let texts: Vec<String> = out.iter().map(text).collect();
        assert_eq!(texts, vec!["hello big", "wide world", "here"]);
        assert_eq!(out[0].spans[0].style, bold);
    }

    #[test]
    fn indents_continuations_and_breaks_long_words() {
        let line = Line::from("abcdefghijkl mn");
        let out = wrap_line(line, 6, 2);
        let texts: Vec<String> = out.iter().map(text).collect();
        assert_eq!(texts, vec!["abcdef", "  ghij", "  kl", "  mn"]);
    }

    #[test]
    fn zero_width_disables_wrapping() {
        let line = Line::from("a long line that would wrap");
        assert_eq!(wrap_line(line.clone(), 0, 0), vec![line]);
    }
}
