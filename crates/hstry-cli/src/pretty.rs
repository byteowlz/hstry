//! Pretty terminal output formatting for hstry CLI.

use chrono::{DateTime, Utc};
use console::{Style, Term, style};
use hstry_core::models::SearchHit;

/// Icons - Nerd Font or ASCII fallback
struct Icons {
    folder: &'static str,
    clock: &'static str,
    host: &'static str,
}

/// Conversation display data for list output.
#[derive(Debug, Clone)]
pub struct ConversationDisplay {
    pub id: uuid::Uuid,
    pub source_id: String,
    pub workspace: Option<String>,
    pub created_at: DateTime<Utc>,
    pub title: String,
}

impl Icons {
    fn detect() -> Self {
        if Self::has_nerd_font() {
            Self {
                folder: "\u{f07b}", // nf-fa-folder
                clock: "\u{f017}",  // nf-fa-clock_o
                host: "\u{f108}",   // nf-fa-desktop
            }
        } else {
            Self {
                folder: "",
                clock: "",
                host: "@",
            }
        }
    }

    fn has_nerd_font() -> bool {
        if let Ok(val) = std::env::var("NERD_FONT") {
            return val != "0" && !val.is_empty();
        }
        if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
            let modern = [
                "WezTerm",
                "Alacritty",
                "kitty",
                "iTerm.app",
                "Hyper",
                "ghostty",
            ];
            if modern.iter().any(|t| term_prog.contains(t)) {
                return true;
            }
        }
        if std::env::var("STARSHIP_SESSION_KEY").is_ok() {
            return true;
        }
        false
    }
}

/// Terminal width for formatting, with fallback.
fn term_width() -> usize {
    Term::stdout().size().1 as usize
}

/// Format a short relative time string.
pub fn relative_time_short(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    if duration.num_minutes() < 60 {
        return format!("{}m", duration.num_minutes().max(1));
    }
    if duration.num_hours() < 24 {
        return format!("{}h", duration.num_hours());
    }
    if duration.num_days() < 7 {
        return format!("{}d", duration.num_days());
    }
    if duration.num_weeks() < 8 {
        return format!("{}w", duration.num_weeks());
    }
    dt.format("%Y-%m-%d").to_string()
}

/// Create a compact score bar with score embedded.
fn score_bar(score: f32) -> String {
    let abs_score = score.abs();
    let clamped = (abs_score - 5.0).clamp(0.0, 10.0);
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let filled = ((clamped / 10.0) * 8.0) as usize; // 8 chars for the bar portion

    // Use thin bar characters: ▰ (filled) ▱ (empty)
    let score_str = format!("{:>4.1}", abs_score);
    let bar = "▰".repeat(filled) + &"▱".repeat(8 - filled);
    format!("{} {}", bar, score_str)
}

/// Style for role.
fn role_style(role: &str) -> Style {
    match role.to_lowercase().as_str() {
        "user" => Style::new().cyan(),
        "assistant" => Style::new().green(),
        "system" => Style::new().magenta(),
        "tool" => Style::new().yellow(),
        _ => Style::new().white(),
    }
}

/// Decode HTML entities and clean up snippet text.
fn clean_snippet(s: &str) -> String {
    let mut result = s.to_string();

    // Decode HTML entities
    result = result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x22;", "\"")
        .replace("&#34;", "\"");

    // Collapse whitespace: newlines, tabs, multiple spaces -> single space
    result = result
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();

    // Collapse multiple spaces
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }

    result.trim().to_string()
}

/// Colorize <b> tags in snippet.
fn colorize_snippet(s: &str) -> String {
    if !console::colors_enabled() {
        return s.replace("<b>", "").replace("</b>", "");
    }

    let mut result = s.replace("<b>", "\x1b[1;33m").replace("</b>", "\x1b[0m");

    // Clean up broken tags
    result = result.replace("<b", "").replace("</b", "\x1b[0m");

    if result.contains("\x1b[1;33m") && !result.ends_with("\x1b[0m") {
        result.push_str("\x1b[0m");
    }

    result
}

/// Shorten a path for display.
pub fn short_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let parts: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        return format!("...{}", &path[path.len().saturating_sub(max_len - 3)..]);
    }
    // Show last 2 components
    let tail = parts[parts.len() - 2..].join("/");
    if tail.len() + 4 <= max_len {
        format!(".../{tail}")
    } else {
        format!("...{}", &path[path.len().saturating_sub(max_len - 3)..])
    }
}

/// Pad a line to width and add right border.
fn pad_line(content: &str, width: usize) -> String {
    let visible_len = console::measure_text_width(content);
    // width includes both borders, so inner content width is width - 2
    // but left border is already in content, so we need width - 1 for content + right border
    let target = width - 1;
    let padding = target.saturating_sub(visible_len);
    format!("{}{}{}", content, " ".repeat(padding), style("│").dim())
}

fn truncate_middle(value: &str, max_len: usize) -> String {
    let length = value.chars().count();
    if length <= max_len {
        return value.to_string();
    }
    if max_len <= 3 {
        return "...".to_string();
    }

    let head_len = max_len / 2 - 1;
    let tail_len = max_len - head_len - 3;
    let head: String = value.chars().take(head_len).collect();
    let tail: String = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}...{tail}")
}

/// Print search results in a compact format.
pub fn print_search_results(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("{}", style("No results found.").dim());
        return;
    }

    let width = term_width();
    let inner = width - 2;
    let header_text = format!(" Found {} result(s) ", hits.len());
    let padding = inner.saturating_sub(header_text.len());

    // Header
    println!(
        "{}{}{}",
        style("╭").dim(),
        style("─".repeat(inner)).dim(),
        style("╮").dim()
    );
    println!(
        "{}{}{}{}",
        style("│").dim(),
        style(&header_text).bold(),
        " ".repeat(padding),
        style("│").dim()
    );
    println!(
        "{}{}{}",
        style("├").dim(),
        style("─".repeat(inner)).dim(),
        style("┤").dim()
    );

    for (i, hit) in hits.iter().enumerate() {
        // Separator between items
        if i > 0 {
            println!(
                "{}{}{}",
                style("├").dim(),
                style("─".repeat(inner)).dim(),
                style("┤").dim()
            );
        }

        // Line 1: metadata (score bar with score, role, adapter, workspace, date)
        let icons = Icons::detect();
        let bar = score_bar(hit.score);
        let role_str = hit.role.to_string();
        let role = role_style(&role_str).apply_to(&role_str);
        let adapter = style(&hit.source_adapter).cyan();
        let date = relative_time_short(hit.conv_created_at);

        let ws_max = width.saturating_sub(60).max(20);
        let ws = hit
            .workspace
            .as_ref()
            .map(|w| format!("{} {}", icons.folder, short_path(w, ws_max)))
            .unwrap_or_default();

        let host_str = hit
            .host
            .as_ref()
            .map(|h| format!("{} {} ", icons.host, h))
            .unwrap_or_default();

        let date_str = format!("{} {}", icons.clock, date);

        let line1 = format!(
            "{} {} {} {} {} {}{}",
            style("│").dim(),
            style(bar).yellow(),
            role,
            adapter,
            style(&ws).dim(),
            style(host_str).dim(),
            style(date_str).dim().italic()
        );
        println!("{}", pad_line(&line1, width));

        // Line 2: title (if present, truncated)
        if let Some(title) = &hit.title {
            let clean: String = title
                .chars()
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            let max_title = inner.saturating_sub(2);
            let display = if clean.len() > max_title {
                format!("{}...", &clean[..max_title - 3])
            } else {
                clean
            };
            let line2 = format!("{} {}", style("│").dim(), style(display).bold());
            println!("{}", pad_line(&line2, width));
        }

        // Line 3: snippet (cleaned, single line, highlighted)
        let snippet = clean_snippet(&hit.snippet);
        let snippet = colorize_snippet(&snippet);
        let max_snippet = inner.saturating_sub(2);
        let display = if console::measure_text_width(&snippet) > max_snippet {
            // Truncate by visible width
            let mut truncated = String::new();
            let mut vis_len = 0;
            let mut in_escape = false;
            for c in snippet.chars() {
                if c == '\x1b' {
                    in_escape = true;
                }
                if in_escape {
                    truncated.push(c);
                    if c == 'm' {
                        in_escape = false;
                    }
                } else {
                    if vis_len >= max_snippet - 3 {
                        break;
                    }
                    truncated.push(c);
                    vis_len += 1;
                }
            }
            truncated + "..."
        } else {
            snippet
        };
        let line3 = format!("{} {}", style("│").dim(), display);
        println!("{}", pad_line(&line3, width));
    }

    // Footer
    println!(
        "{}{}{}",
        style("╰").dim(),
        style("─".repeat(inner)).dim(),
        style("╯").dim()
    );
}

/// Print conversations in a nice table format.
pub fn print_conversations(conversations: &[ConversationDisplay]) {
    if conversations.is_empty() {
        println!("{}", style("No conversations found.").dim());
        return;
    }

    let width = term_width();
    let inner = width - 2;
    let header_text = format!(" {} conversation(s) ", conversations.len());
    let padding = inner.saturating_sub(header_text.len());

    // Header
    println!(
        "{}{}{}",
        style("╭").dim(),
        style("─".repeat(inner)).dim(),
        style("╮").dim()
    );
    println!(
        "{}{}{}{}",
        style("│").dim(),
        style(&header_text).bold(),
        " ".repeat(padding),
        style("│").dim()
    );
    println!(
        "{}{}{}",
        style("├").dim(),
        style("─".repeat(inner)).dim(),
        style("┤").dim()
    );

    let icons = Icons::detect();

    for (i, conv) in conversations.iter().enumerate() {
        // Separator between items
        if i > 0 {
            println!(
                "{}{}{}",
                style("├").dim(),
                style("─".repeat(inner)).dim(),
                style("┤").dim()
            );
        }

        // Single line: title | workdir | time | agent | id
        let agent = style(&conv.source_id).cyan();
        let date = relative_time_short(conv.created_at);
        let date_str = format!("{} {}", icons.clock, date);
        let id_short = conv.id.to_string()[..8].to_string();

        let workdir_raw = conv
            .workspace
            .as_ref()
            .map(|w| format!("{} {}", icons.folder, w))
            .unwrap_or_else(|| "-".to_string());

        let clean_title: String = conv
            .title
            .chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let fixed_width = console::measure_text_width(&format!(
            "{} | {} | {}",
            date_str, conv.source_id, id_short
        ));
        let available = inner.saturating_sub(fixed_width).saturating_sub(8);
        let title_max = (available * 2 / 3).max(12);
        let workdir_max = available.saturating_sub(title_max).max(10);

        let title_display = if clean_title.chars().count() > title_max {
            format!(
                "{}...",
                clean_title
                    .chars()
                    .take(title_max.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            clean_title
        };
        let workdir_display = truncate_middle(&workdir_raw, workdir_max);

        let line = format!(
            "{} {} | {} | {} | {} | {}",
            style("│").dim(),
            style(title_display).bold(),
            style(workdir_display).dim(),
            style(date_str).dim().italic(),
            agent,
            style(id_short).dim()
        );
        println!("{}", pad_line(&line, width));
    }

    // Footer
    println!(
        "{}{}{}",
        style("╰").dim(),
        style("─".repeat(inner)).dim(),
        style("╯").dim()
    );
}

/// Print search results in TSV format (for piping/scripting).
#[expect(dead_code)]
pub fn print_search_results_compact(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("No results found.");
        return;
    }

    for hit in hits {
        let score = hit.score.abs();
        let title = hit.title.as_deref().unwrap_or("-");
        let ws = hit.workspace.as_deref().unwrap_or("-");
        println!(
            "{score:.1}\t{role}\t{adapter}\t{ws}\t{title}",
            role = hit.role,
            adapter = hit.source_adapter,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_time_short() {
        let now = Utc::now();
        assert_eq!(relative_time_short(now), "1m");
    }

    #[test]
    fn test_score_bar() {
        let bar = score_bar(10.0);
        assert!(bar.contains('▰'));
        assert!(bar.contains("10.0"));
    }

    #[test]
    fn test_clean_snippet() {
        let input = "hello\n\t  world   foo";
        assert_eq!(clean_snippet(input), "hello world foo");
    }

    #[test]
    fn test_short_path() {
        assert_eq!(short_path("/home/user/code", 20), "/home/user/code");
        let short = short_path("/home/user/very/long/path/to/project", 20);
        assert!(short.starts_with("..."));
        assert!(short.len() <= 20);
    }
}
