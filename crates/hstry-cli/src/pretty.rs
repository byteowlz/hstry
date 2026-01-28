//! Pretty terminal output formatting for hstry CLI.

use chrono::{DateTime, Utc};
use console::{style, Style, Term};
use hstry_core::models::SearchHit;

/// Terminal width for formatting, with fallback.
fn term_width() -> usize {
    Term::stdout().size().1 as usize
}

/// Format a short relative time string.
fn relative_time_short(dt: DateTime<Utc>) -> String {
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

/// Create a compact score bar.
fn score_bar(score: f32) -> String {
    let abs_score = score.abs();
    let clamped = (abs_score - 5.0).clamp(0.0, 10.0);
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let filled = ((clamped / 10.0) * 5.0) as usize; // 5 chars max
    "█".repeat(filled) + &"░".repeat(5 - filled)
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

    let mut result = s
        .replace("<b>", "\x1b[1;33m")
        .replace("</b>", "\x1b[0m");

    // Clean up broken tags
    result = result.replace("<b", "").replace("</b", "\x1b[0m");

    if result.contains("\x1b[1;33m") && !result.ends_with("\x1b[0m") {
        result.push_str("\x1b[0m");
    }

    result
}

/// Shorten a path for display.
fn short_path(path: &str, max_len: usize) -> String {
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

/// Print search results in a compact format.
pub fn print_search_results(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("{}", style("No results found.").dim());
        return;
    }

    let width = term_width().min(120);
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

        // Line 1: metadata (score, role, adapter, workspace, date) - all on one line
        let bar = score_bar(hit.score);
        let score = hit.score.abs();
        let role_str = hit.role.to_string();
        let role = role_style(&role_str).apply_to(&role_str);
        let adapter = style(&hit.source_adapter).cyan();
        let date = relative_time_short(hit.conv_created_at);

        let ws = hit
            .workspace
            .as_ref()
            .map(|w| short_path(w, 35))
            .unwrap_or_default();

        let host_str = hit
            .host
            .as_ref()
            .map(|h| format!("@{h}"))
            .unwrap_or_default();

        println!(
            "{} {} {:>4.1} {} {} {} {} {}",
            style("│").dim(),
            style(bar).yellow(),
            style(score).dim(),
            role,
            adapter,
            style(&ws).dim(),
            style(host_str).dim(),
            style(date).dim().italic()
        );

        // Line 2: title (if present, truncated)
        if let Some(title) = &hit.title {
            let clean: String = title
                .chars()
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            let max_title = inner.saturating_sub(5);
            let display = if clean.len() > max_title {
                format!("{}...", &clean[..max_title - 3])
            } else {
                clean
            };
            println!(
                "{}  {} {}",
                style("│").dim(),
                style("▶").cyan(),
                style(display).bold()
            );
        }

        // Line 3: snippet (cleaned, single line, highlighted)
        let snippet = clean_snippet(&hit.snippet);
        let snippet = colorize_snippet(&snippet);
        let max_snippet = inner.saturating_sub(5);
        let display = if snippet.len() > max_snippet {
            let visible_len = console::measure_text_width(&snippet);
            if visible_len > max_snippet {
                format!("{}...", &snippet[..max_snippet.saturating_sub(3)])
            } else {
                snippet
            }
        } else {
            snippet
        };
        println!("{}    {}", style("│").dim(), display);
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
        assert!(bar.contains('█'));
        assert_eq!(bar.chars().count(), 5);
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
