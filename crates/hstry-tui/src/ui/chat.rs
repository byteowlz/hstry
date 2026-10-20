//! The transcript: speaker blocks built from canonical message parts.
//!
//! Prose is rendered as markdown under a speaker header. Thinking, tool calls
//! and tool results are rendered as what they are (dim reasoning, a one-line
//! call, indented output) instead of being flattened into prose, and tool
//! output is attached to the turn that requested it rather than to "you".

use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use hstry_core::models::{Message, MessageRole};

use crate::images::{ImageEntry, extract_images};
use crate::markdown::{highlight_lines, render_markdown, truncate_str};
use crate::state::{App, Focus, relative_time, shorten_workspace};
use crate::theme::{Token, adapter_color};
use crate::ui::{THEME, panel};

/// Width of the speaker gutter drawn left of prose lines.
const GUTTER: usize = 2;
/// Collapsed previews: lines of thinking / tool output shown before "… more".
const THINKING_PREVIEW: usize = 2;
const OUTPUT_PREVIEW: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Speaker {
    User,
    Assistant,
}

const fn speaker_color(speaker: Speaker) -> Color {
    match speaker {
        Speaker::User => Color::Green,
        Speaker::Assistant => Color::Blue,
    }
}

const fn speaker_name(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::User => "you",
        Speaker::Assistant => "assistant",
    }
}

/// One renderable piece of a message.
enum Piece {
    Text(String),
    Thinking(String),
    ToolCall { name: String, hint: String },
    ToolResult { output: String, is_error: bool },
}

fn pieces(msg: &Message) -> Vec<Piece> {
    let parts = msg.parts_json.as_array().filter(|p| !p.is_empty());
    let Some(parts) = parts else {
        let text = msg.content.trim();
        if text.is_empty() {
            return Vec::new();
        }
        return vec![if msg.role == MessageRole::Tool {
            Piece::ToolResult {
                output: text.to_string(),
                is_error: false,
            }
        } else {
            Piece::Text(text.to_string())
        }];
    };

    let mut out = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(t) = part.get("text").and_then(Value::as_str).map(str::trim)
                    && !t.is_empty()
                {
                    // A text part on a tool message is tool output, not prose.
                    out.push(if msg.role == MessageRole::Tool {
                        Piece::ToolResult {
                            output: t.to_string(),
                            is_error: false,
                        }
                    } else {
                        Piece::Text(t.to_string())
                    });
                }
            }
            "thinking" => {
                if let Some(t) = part.get("text").and_then(Value::as_str).map(str::trim)
                    && !t.is_empty()
                {
                    out.push(Piece::Thinking(t.to_string()));
                }
            }
            "tool_call" => {
                let name = part
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                out.push(Piece::ToolCall {
                    hint: tool_hint(part.get("input")),
                    name,
                });
            }
            "tool_result" => {
                out.push(Piece::ToolResult {
                    output: value_text(part.get("output")),
                    is_error: part
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
            _ => {}
        }
    }
    if out.is_empty() && !msg.content.trim().is_empty() {
        out.push(Piece::Text(msg.content.trim().to_string()));
    }
    out
}

/// The most recognizable argument of a tool call, on one line.
fn tool_hint(input: Option<&Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    if let Some(obj) = input.as_object() {
        for key in [
            "command",
            "cmd",
            "file_path",
            "path",
            "filePath",
            "pattern",
            "query",
            "url",
            "code",
            "prompt",
            "description",
        ] {
            match obj.get(key) {
                Some(Value::String(s)) if !s.trim().is_empty() => return one_line(s),
                Some(Value::Array(items)) if !items.is_empty() => {
                    let joined: Vec<String> = items
                        .iter()
                        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                        .collect();
                    return one_line(&joined.join(" "));
                }
                _ => {}
            }
        }
    }
    match input {
        Value::String(s) => one_line(s),
        Value::Null => String::new(),
        other => one_line(&other.to_string()),
    }
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map_or_else(|| item.to_string(), str::to_owned)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build the full transcript as styled lines plus any images found in parts.
///
/// `width` is the content width available for wrapping (`0` disables
/// wrapping). `expand` shows thinking and tool output in full.
pub fn build_chat_lines(
    messages: &[Message],
    highlight: Option<&str>,
    width: usize,
    expand: bool,
) -> (Vec<Line<'static>>, Vec<ImageEntry>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<ImageEntry> = Vec::new();
    let mut last_model: Option<&str> = None;
    let mut current: Option<Speaker> = None;
    let mut prev_system = false;
    let body_width = width.saturating_sub(GUTTER);

    for msg in messages {
        if msg.role == MessageRole::System {
            if !lines.is_empty() && !prev_system {
                lines.push(Line::from(""));
            }
            let text = one_line(&msg.content);
            if !text.is_empty() {
                let notice = Line::from(vec![
                    Span::styled("· ", THEME.fg(Token::Warn)),
                    Span::styled(text, THEME.fg(Token::Muted)),
                ]);
                lines.extend(wrap_line(notice, width, GUTTER));
            }
            prev_system = true;
            current = None;
            continue;
        }
        prev_system = false;

        let pieces = pieces(msg);
        if pieces.is_empty() {
            continue;
        }
        let only_tool_output = pieces.iter().all(|p| matches!(p, Piece::ToolResult { .. }));
        // Tool output belongs to the turn that asked for it.
        let speaker = if only_tool_output && current.is_some() {
            current.unwrap_or(Speaker::Assistant)
        } else if msg.role == MessageRole::User {
            Speaker::User
        } else {
            Speaker::Assistant
        };

        if current != Some(speaker) {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            let color = speaker_color(speaker);
            let mut header = vec![
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(speaker_name(speaker), Style::default().fg(color).bold()),
            ];
            if let Some(ts) = msg.created_at {
                let local: DateTime<Local> = ts.into();
                header.push(Span::styled(
                    format!("  {}", local.format("%H:%M")),
                    THEME.fg(Token::Muted),
                ));
            }
            if speaker == Speaker::Assistant
                && let Some(model) = msg.model.as_deref()
                && last_model != Some(model)
            {
                header.push(Span::styled(format!("  {model}"), THEME.fg(Token::Muted)));
                last_model = Some(model);
            }
            lines.push(Line::from(header));
            current = Some(speaker);
        } else if pieces.iter().any(|p| matches!(p, Piece::Text(_))) {
            // A new prose message from the same speaker gets breathing room.
            lines.push(Line::from(""));
        }

        let gutter = Span::styled("▏ ", Style::default().fg(speaker_color(speaker)));
        for piece in pieces {
            match piece {
                Piece::Text(text) => {
                    let role = if speaker == Speaker::User {
                        MessageRole::User
                    } else {
                        MessageRole::Assistant
                    };
                    for line in render_markdown(&text, &role, highlight) {
                        for wrapped in wrap_line(line, body_width, 0) {
                            let mut spans = vec![gutter.clone()];
                            spans.extend(wrapped.spans);
                            lines.push(Line { spans, ..wrapped });
                        }
                    }
                }
                Piece::Thinking(text) => {
                    push_thinking(&mut lines, &text, width, expand, highlight);
                }
                Piece::ToolCall { name, hint } => {
                    let mut spans = vec![
                        Span::styled("  ⚙ ", THEME.fg(Token::Accent)),
                        Span::styled(name, THEME.fg_bold(Token::Primary)),
                    ];
                    if !hint.is_empty() {
                        let room = width.saturating_sub(spans_width(&spans) + 2).max(12);
                        spans.push(Span::styled(
                            format!("  {}", truncate_str(&hint, room)),
                            THEME.fg(Token::Muted),
                        ));
                    }
                    let line = Line::from(spans);
                    lines.extend(highlight.map_or_else(
                        || vec![line.clone()],
                        |q| highlight_lines(vec![line.clone()], q),
                    ));
                }
                Piece::ToolResult { output, is_error } => {
                    push_tool_output(&mut lines, &output, is_error, width, expand, highlight);
                }
            }
        }

        for entry in extract_images(&msg.parts_json) {
            images.push(entry.clone());
            lines.push(Line::from(vec![
                gutter.clone(),
                Span::styled(
                    format!("▣ image {}: {}  (i to view)", images.len(), entry.label),
                    Style::default().fg(Color::Cyan),
                ),
            ]));
        }
    }

    (lines, images)
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

fn push_thinking(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    expand: bool,
    highlight: Option<&str>,
) {
    let style = THEME
        .fg(Token::Muted)
        .add_modifier(Modifier::ITALIC | Modifier::DIM);
    let prefix = "  ∴ ";
    let mut out: Vec<Line<'static>> = Vec::new();
    for (i, raw) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let lead = if i == 0 { prefix } else { "    " };
        let line = Line::from(vec![
            Span::styled(lead, THEME.fg(Token::Muted)),
            Span::styled(raw.trim().to_string(), style),
        ]);
        out.extend(wrap_line(line, width, 4));
    }
    let total = out.len();
    if !expand && total > THINKING_PREVIEW {
        out.truncate(THINKING_PREVIEW);
        out.push(Line::from(Span::styled(
            format!(
                "    … {} more lines of thinking  (t)",
                total - THINKING_PREVIEW
            ),
            THEME.fg(Token::Muted),
        )));
    }
    lines.extend(match highlight {
        Some(q) => highlight_lines(out, q),
        None => out,
    });
}

fn push_tool_output(
    lines: &mut Vec<Line<'static>>,
    output: &str,
    is_error: bool,
    width: usize,
    expand: bool,
    highlight: Option<&str>,
) {
    let text = output.trim_end();
    let (mark, mark_style, body_style) = if is_error {
        (
            "  ✗ ",
            THEME.fg_bold(Token::Danger),
            THEME.fg(Token::Danger),
        )
    } else {
        ("  └ ", THEME.fg(Token::Muted), THEME.fg(Token::Muted))
    };
    if text.trim().is_empty() {
        lines.push(Line::from(vec![
            Span::styled(mark, mark_style),
            Span::styled(
                if is_error {
                    "error (no output)"
                } else {
                    "(no output)"
                },
                body_style,
            ),
        ]));
        return;
    }
    let source: Vec<&str> = text.lines().collect();
    let total = source.len();
    let shown = if expand {
        total
    } else {
        total.min(OUTPUT_PREVIEW)
    };
    let mut out: Vec<Line<'static>> = Vec::new();
    for (i, raw) in source.iter().take(shown).enumerate() {
        let lead = if i == 0 { mark } else { "    " };
        let room = width.saturating_sub(4).max(8);
        // Tool output is not prose: keep each line, cut instead of reflowing.
        let body = if expand {
            (*raw).to_string()
        } else {
            truncate_str(raw, room)
        };
        let line = Line::from(vec![
            Span::styled(lead, if i == 0 { mark_style } else { body_style }),
            Span::styled(body.replace('\t', "    "), body_style),
        ]);
        if expand {
            out.extend(wrap_line(line, width, 4));
        } else {
            out.push(line);
        }
    }
    if shown < total {
        out.push(Line::from(Span::styled(
            format!("    … {} more lines  (t)", total - shown),
            THEME.fg(Token::Muted),
        )));
    }
    lines.extend(match highlight {
        Some(q) => highlight_lines(out, q),
        None => out,
    });
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
    let active = app.focus == Focus::Transcript;

    let title = app.cursor_session().map_or_else(
        || "transcript".to_string(),
        |s| truncate_str(s.title(), area.width.saturating_sub(6) as usize),
    );
    let mut block = panel(&title, active);

    if let Some(session) = app.cursor_session() {
        let mut meta: Vec<Span<'static>> = vec![
            Span::styled(" ● ", Style::default().fg(adapter_color(&session.adapter))),
            Span::styled(session.adapter.clone(), THEME.fg(Token::Muted)),
        ];
        let mut push = |text: String| {
            meta.push(Span::styled(" · ", THEME.fg(Token::Muted)));
            meta.push(Span::styled(text, THEME.fg(Token::Muted)));
        };
        if let Some(model) = &session.conv.model {
            push(model.clone());
        }
        if let Some(ws) = &session.conv.workspace {
            push(shorten_workspace(ws));
        }
        push(relative_time(session.last_active()));
        push(format!(
            "{} msgs",
            session.message_count.max(app.messages.len() as i64)
        ));
        if !app.chat_images.is_empty() {
            push(format!("{} images", app.chat_images.len()));
        }
        meta.push(Span::raw(" "));
        block = block.title_bottom(Line::from(meta));
    }

    let inner = block.inner(area);
    f.render_widget(block, area);
    // Leave a column for the scrollbar.
    let text_area = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };
    app.chat_height = text_area.height;
    if usize::from(text_area.width) != app.chat_wrap_width {
        app.chat_wrap_width = usize::from(text_area.width);
        app.rebuild_chat_lines();
    }

    if app.chat_lines.is_empty() {
        let hint = if app.sessions.is_empty() {
            "Nothing here yet. Run `hstry sync` to import your agent sessions."
        } else if app.rows.is_empty() {
            "No session matches the query."
        } else {
            "Select a session to read it here."
        };
        let empty = Paragraph::new(hint)
            .style(THEME.fg(Token::Muted))
            .wrap(Wrap { trim: true });
        f.render_widget(empty, text_area);
        return;
    }

    let max_scroll = app
        .chat_lines
        .len()
        .saturating_sub(usize::from(text_area.height).max(1));
    app.chat_scroll = app.chat_scroll.min(max_scroll);
    let scroll = u16::try_from(app.chat_scroll).unwrap_or(u16::MAX);

    let paragraph = Paragraph::new(app.chat_lines.clone()).scroll((scroll, 0));
    f.render_widget(paragraph, text_area);

    if app.chat_lines.len() > usize::from(text_area.height) {
        let mut state = ScrollbarState::new(max_scroll).position(app.chat_scroll);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("┃")
            .track_style(THEME.fg(Token::Muted).add_modifier(Modifier::DIM))
            .thumb_style(if active {
                THEME.fg(Token::Accent)
            } else {
                THEME.fg(Token::Muted)
            });
        f.render_stateful_widget(bar, inner.inner(Margin::new(0, 0)), &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn message(role: MessageRole, content: &str, parts: Value) -> Message {
        serde_json::from_value(json!({
            "id": Uuid::nil(),
            "conversation_id": Uuid::nil(),
            "idx": 0,
            "role": role,
            "content": content,
            "parts_json": parts,
            "created_at": null,
            "model": null,
            "tokens": null,
            "cost_usd": null,
            "metadata": {}
        }))
        .expect("message fixture")
    }

    #[test]
    fn tool_output_attaches_to_the_assistant_turn() {
        let msgs = vec![
            message(
                MessageRole::User,
                "find it",
                json!([{"type":"text","text":"find it"}]),
            ),
            message(
                MessageRole::Assistant,
                "",
                json!([
                    {"type":"thinking","text":"look first"},
                    {"type":"tool_call","toolCallId":"t1","name":"Bash","input":{"command":"rg retry"}}
                ]),
            ),
            message(
                MessageRole::User,
                "src/a.rs:1",
                json!([{"type":"tool_result","toolCallId":"t1","output":"src/a.rs:1"}]),
            ),
        ];
        let (lines, _) = build_chat_lines(&msgs, None, 80, false);
        let all: Vec<String> = lines.iter().map(text).collect();
        let you_headers = all.iter().filter(|l| l.contains("you")).count();
        assert_eq!(
            you_headers, 1,
            "tool output must not be labelled as you: {all:#?}"
        );
        assert!(
            all.iter()
                .any(|l| l.contains("⚙ Bash") && l.contains("rg retry")),
            "{all:#?}"
        );
        assert!(all.iter().any(|l| l.contains("└ src/a.rs:1")), "{all:#?}");
        assert!(all.iter().any(|l| l.contains("∴ look first")), "{all:#?}");
    }

    #[test]
    fn long_tool_output_is_collapsed_until_expanded() {
        let output: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let msgs = vec![
            message(
                MessageRole::Assistant,
                "",
                json!([{"type":"tool_call","toolCallId":"t","name":"ls","input":{}}]),
            ),
            message(
                MessageRole::Tool,
                &output,
                json!([{"type":"tool_result","toolCallId":"t","output":output}]),
            ),
        ];
        let (collapsed, _) = build_chat_lines(&msgs, None, 80, false);
        let (expanded, _) = build_chat_lines(&msgs, None, 80, true);
        assert!(
            collapsed
                .iter()
                .map(text)
                .any(|l| l.contains("14 more lines"))
        );
        assert!(expanded.len() > collapsed.len());
        assert!(expanded.iter().map(text).any(|l| l.contains("line 19")));
    }

    #[test]
    fn errored_tool_results_are_marked() {
        let msgs = vec![message(
            MessageRole::Tool,
            "boom",
            json!([{"type":"tool_result","toolCallId":"t","output":"boom","isError":true}]),
        )];
        let (lines, _) = build_chat_lines(&msgs, None, 80, false);
        assert!(lines.iter().map(text).any(|l| l.contains("✗ boom")));
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
