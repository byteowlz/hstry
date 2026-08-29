//! Markdown and tool-output rendering into styled ratatui lines.

use pulldown_cmark::{Event as MdEvent, Options, Parser as MdParser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
};

use hstry_core::models::MessageRole;

/// Render markdown content to styled ratatui Lines.
pub fn render_markdown(
    content: &str,
    role: &MessageRole,
    highlight: Option<&str>,
) -> Vec<Line<'static>> {
    if *role == MessageRole::Tool
        && let Some(lines) = try_format_tool_output(content)
    {
        return lines;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];

    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut list_depth: usize = 0;
    let mut in_heading = false;
    let mut heading_level = 0;

    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = MdParser::new_ext(content, options);

    for event in parser {
        match event {
            MdEvent::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut lines, &mut current_spans);
                    in_heading = true;
                    heading_level = level as usize;
                    let style = Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD);
                    style_stack.push(style);
                }
                Tag::CodeBlock(kind) => {
                    flush_line(&mut lines, &mut current_spans);
                    in_code_block = true;
                    code_block_lang = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                            let l = lang.to_string();
                            if l.is_empty() { None } else { Some(l) }
                        }
                        pulldown_cmark::CodeBlockKind::Indented => None,
                    };
                    code_block_lines.clear();
                }
                Tag::List(_) => {
                    list_depth += 1;
                }
                Tag::Item => {
                    flush_line(&mut lines, &mut current_spans);
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    current_spans.push(Span::styled(
                        format!("{indent}• "),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                Tag::Emphasis => {
                    let style = current_style(&style_stack).add_modifier(Modifier::ITALIC);
                    style_stack.push(style);
                }
                Tag::Strong => {
                    let style = current_style(&style_stack).add_modifier(Modifier::BOLD);
                    style_stack.push(style);
                }
                Tag::Strikethrough => {
                    let style = current_style(&style_stack).add_modifier(Modifier::CROSSED_OUT);
                    style_stack.push(style);
                }
                Tag::Link { dest_url, .. } => {
                    let style = Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED);
                    style_stack.push(style);
                    let _ = dest_url;
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans);
                    let style = Style::default().fg(Color::DarkGray);
                    style_stack.push(style);
                    current_spans.push(Span::styled("▎ ", Style::default().fg(Color::DarkGray)));
                }
                _ => {}
            },
            MdEvent::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    flush_line(&mut lines, &mut current_spans);
                    in_heading = false;
                    heading_level = 0;
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let lang = code_block_lang.take().unwrap_or_default();
                    render_code_block(&mut lines, &lang, &code_block_lines);
                    code_block_lines.clear();
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    if list_depth == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    flush_line(&mut lines, &mut current_spans);
                }
                _ => {}
            },
            MdEvent::Text(text) => {
                if in_code_block {
                    for line in text.lines() {
                        code_block_lines.push(line.to_string());
                    }
                } else {
                    let style = current_style(&style_stack);
                    if in_heading && current_spans.is_empty() {
                        let prefix = "#".repeat(heading_level);
                        current_spans.push(Span::styled(
                            format!("{prefix} "),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            MdEvent::Code(code) => {
                current_spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Yellow).bg(Color::Black),
                ));
            }
            MdEvent::SoftBreak if !in_code_block => {
                current_spans.push(Span::raw(" "));
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Rule => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(Line::from("─".repeat(24)).fg(Color::DarkGray));
            }
            _ => {}
        }
    }

    flush_line(&mut lines, &mut current_spans);

    while lines.last().is_some_and(|l| l.spans.is_empty()) {
        lines.pop();
    }

    if let Some(term) = highlight.filter(|t| !t.trim().is_empty()) {
        highlight_lines(lines, term)
    } else {
        lines
    }
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}

fn render_code_block(lines: &mut Vec<Line<'static>>, lang: &str, code_lines: &[String]) {
    let display_lang = if lang.is_empty() { "code" } else { lang };
    let collapsed = code_lines.len() > 16;
    let code_style = Style::default().fg(Color::Gray).bg(Color::Black);
    let gutter = Span::styled(" ", Style::default().bg(Color::Black));

    lines.push(Line::from(vec![
        Span::styled("╭ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            display_lang.to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            if collapsed {
                format!(" · {} lines", code_lines.len())
            } else {
                String::new()
            },
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    let shown: &[String] = if collapsed {
        &code_lines[..8]
    } else {
        code_lines
    };
    for line in shown {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            gutter.clone(),
            Span::styled(line.clone(), code_style),
        ]));
    }
    if collapsed {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            Span::styled("…", Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "╰",
        Style::default().fg(Color::DarkGray),
    )));
}

pub fn truncate_str(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(1);
        let truncated: String = s.chars().take(keep).collect();
        format!("{truncated}…")
    }
}

fn highlight_lines(lines: Vec<Line<'static>>, term: &str) -> Vec<Line<'static>> {
    let needle = term.to_lowercase();
    lines
        .into_iter()
        .map(|line| highlight_line(line, &needle))
        .collect()
}

fn highlight_line(line: Line<'static>, needle: &str) -> Line<'static> {
    if needle.is_empty() {
        return line;
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for span in line.spans {
        highlight_span_into(span, needle, &mut spans);
    }

    Line {
        spans,
        alignment: line.alignment,
        style: line.style,
    }
}

fn highlight_span_into(span: Span<'static>, needle: &str, out: &mut Vec<Span<'static>>) {
    let text = span.content.as_ref();
    let lower = text.to_lowercase();

    if !lower.contains(needle) {
        out.push(span);
        return;
    }

    let mut rest = text;
    let mut rest_lower = lower.as_str();
    while let Some(idx) = rest_lower.find(needle) {
        let (prefix, after_prefix) = rest.split_at(idx);
        let (_, after_prefix_lower) = rest_lower.split_at(idx);
        if !prefix.is_empty() {
            out.push(Span::styled(prefix.to_string(), span.style));
        }

        let (matched, suffix) = after_prefix.split_at(needle.len());
        let highlight_style = span.style.patch(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        out.push(Span::styled(matched.to_string(), highlight_style));

        rest = suffix;
        rest_lower = &after_prefix_lower[needle.len()..];
    }

    if !rest.is_empty() {
        out.push(Span::styled(rest.to_string(), span.style));
    }
}

// =============================================================================
// Tool Output Formatting
// =============================================================================

fn try_format_tool_output(content: &str) -> Option<Vec<Line<'static>>> {
    if let Some(lines) = try_parse_tool_json(content) {
        return Some(lines);
    }

    if content.starts_with("Exit code:") {
        return Some(format_exit_code_output(content));
    }

    if looks_like_file_listing(content) {
        return Some(format_file_listing(content));
    }

    None
}

fn try_parse_tool_json(content: &str) -> Option<Vec<Line<'static>>> {
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(output) = parsed.get("output").and_then(|v| v.as_str()) {
        if output.starts_with("Success.") || output.starts_with("Updated") {
            lines.push(Line::from(output.lines().next()?.to_string()).fg(Color::Green));

            let files: Vec<&str> = output
                .lines()
                .skip(1)
                .filter(|l| l.starts_with("M ") || l.starts_with("A ") || l.starts_with("D "))
                .collect();

            if !files.is_empty() {
                for f in files.iter().take(5) {
                    lines.push(Line::from(format!("  {f}")).fg(Color::DarkGray));
                }
                if files.len() > 5 {
                    lines.push(
                        Line::from(format!("  … and {} more", files.len() - 5)).fg(Color::DarkGray),
                    );
                }
            }
        } else {
            let output_lines = render_markdown(output, &MessageRole::Tool, None);
            if output_lines.len() > 25 {
                lines.extend(output_lines.into_iter().take(20));
                lines.push(Line::from("… (truncated)").fg(Color::DarkGray));
            } else {
                lines.extend(output_lines);
            }
        }
    }

    if let Some(meta) = parsed.get("metadata")
        && let Some(exit_code) = meta.get("exit_code").and_then(serde_json::Value::as_i64)
        && exit_code != 0
    {
        let time_str = meta
            .get("duration_seconds")
            .and_then(serde_json::Value::as_f64)
            .map(|d| format!(" ({d:.1}s)"))
            .unwrap_or_default();
        lines.insert(
            0,
            Line::from(format!("Exit: {exit_code}{time_str}")).fg(Color::Red),
        );
    }

    if lines.is_empty() { None } else { Some(lines) }
}

fn format_exit_code_output(content: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let content_lines: Vec<&str> = content.lines().collect();

    let mut exit_code = 0;
    let mut wall_time = None;
    let mut output_start = 0;

    for (i, line) in content_lines.iter().enumerate() {
        if line.starts_with("Exit code:") {
            exit_code = line
                .trim_start_matches("Exit code:")
                .trim()
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("Wall time:") {
            wall_time = Some(line.trim_start_matches("Wall time:").trim().to_string());
        } else if line.starts_with("Output:") {
            output_start = i + 1;
            break;
        } else if !line.starts_with("Total output") {
            output_start = i;
            break;
        }
    }

    if exit_code != 0 {
        let time_str = wall_time.map(|t| format!(" ({t})")).unwrap_or_default();
        lines.push(Line::from(format!("Exit: {exit_code}{time_str}")).fg(Color::Red));
    }

    let output: Vec<&str> = content_lines.iter().skip(output_start).copied().collect();

    if looks_like_file_listing_lines(&output) {
        let total = output.len();
        for f in output.iter().take(8) {
            lines.push(Line::from(format!("  {}", shorten_path(f))).fg(Color::DarkGray));
        }
        if total > 8 {
            lines.push(Line::from(format!("  … and {} more", total - 8)).fg(Color::DarkGray));
        }
    } else {
        for line in output.iter().take(20) {
            lines.push(Line::from((*line).to_string()));
        }
        if output.len() > 20 {
            lines.push(
                Line::from(format!("… ({} more lines)", output.len() - 20)).fg(Color::DarkGray),
            );
        }
    }

    lines
}

fn looks_like_file_listing(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().take(10).collect();
    looks_like_file_listing_lines(&lines)
}

fn looks_like_file_listing_lines(lines: &[&str]) -> bool {
    if lines.len() < 3 {
        return false;
    }

    let path_like = lines
        .iter()
        .filter(|l| {
            l.contains('/')
                && (path_has_known_extension(l)
                    || l.contains(':') && l.split(':').next().is_some_and(|p| p.contains('/')))
        })
        .count();

    path_like > lines.len() / 2
}

fn path_has_known_extension(line: &str) -> bool {
    let path_part = line.split(':').next().unwrap_or(line);
    let Some(ext) = std::path::Path::new(path_part)
        .extension()
        .and_then(|value| value.to_str())
    else {
        return false;
    };

    matches!(
        ext.to_ascii_lowercase().as_str(),
        "rs" | "ts" | "py" | "go" | "js" | "tsx" | "json" | "toml" | "md"
    )
}

fn format_file_listing(content: &str) -> Vec<Line<'static>> {
    let file_lines: Vec<&str> = content.lines().collect();
    let total = file_lines.len();
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(format!("Files ({total}):")).fg(Color::Cyan));
    for f in file_lines.iter().take(8) {
        let short = shorten_path(f);
        lines.push(Line::from(format!("  {short}")).fg(Color::DarkGray));
    }
    if total > 8 {
        let remaining = total - 8;
        lines.push(Line::from(format!("  … and {remaining} more")).fg(Color::DarkGray));
    }

    lines
}

fn shorten_path(path: &str) -> String {
    if let Some((file_part, rest)) = path.split_once(':') {
        let short_file: String = file_part
            .rsplit('/')
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("/");
        let rest_truncated = truncate_str(rest, 50);
        format!("{short_file}:{rest_truncated}")
    } else {
        path.rsplit('/')
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("/")
    }
}
