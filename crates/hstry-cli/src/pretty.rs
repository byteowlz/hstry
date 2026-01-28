//! Pretty terminal output formatting for hstry CLI.

use chrono::{DateTime, Utc};
use console::{style, Style, Term};
use hstry_core::models::SearchHit;

/// Icons for terminal output - uses Nerd Font icons if available, ASCII fallbacks otherwise.
struct Icons {
    folder: &'static str,
    host: &'static str,
    calendar: &'static str,
    arrow: &'static str,
    search: &'static str,
    bullet: &'static str,
}

impl Icons {
    fn detect() -> Self {
        if Self::has_nerd_font() {
            Self {
                folder: "\u{f07b} ",  // nf-fa-folder
                host: "\u{f108} ",    // nf-fa-desktop
                calendar: "\u{f073} ", // nf-fa-calendar
                arrow: "\u{f061}",    // nf-fa-arrow_right
                search: "\u{f002} ",  // nf-fa-search
                bullet: "\u{f054}",   // nf-fa-chevron_right
            }
        } else {
            Self {
                folder: "",
                host: "@",
                calendar: "",
                arrow: "->",
                search: "",
                bullet: ">",
            }
        }
    }

    /// Detect if a Nerd Font is likely available.
    /// Checks common environment variables and terminal configurations.
    fn has_nerd_font() -> bool {
        // Check explicit env var (users can set NERD_FONT=1 to force)
        if let Ok(val) = std::env::var("NERD_FONT") {
            return val != "0" && !val.is_empty();
        }

        // Check if TERM_PROGRAM suggests a modern terminal that often has Nerd Fonts
        if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
            let modern_terminals = [
                "WezTerm",
                "Alacritty", 
                "kitty",
                "iTerm.app",
                "Hyper",
                "ghostty",
            ];
            if modern_terminals.iter().any(|t| term_prog.contains(t)) {
                return true;
            }
        }

        // Check TERMINAL_EMULATOR (set by some terminals)
        if let Ok(term_emu) = std::env::var("TERMINAL_EMULATOR") {
            if term_emu.to_lowercase().contains("jetbrains") {
                return true;
            }
        }

        // Check for common Nerd Font indicators in LANG/LC_* (some setups)
        // or presence of STARSHIP (starship prompt users often have nerd fonts)
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

/// Format a relative time string (e.g., "2 days ago", "just now").
fn relative_time(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    if duration.num_seconds() < 60 {
        return "just now".to_string();
    }
    if duration.num_minutes() < 60 {
        let mins = duration.num_minutes();
        return format!("{mins} min{s} ago", s = if mins == 1 { "" } else { "s" });
    }
    if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        return format!("{hours} hour{s} ago", s = if hours == 1 { "" } else { "s" });
    }
    if duration.num_days() < 7 {
        let days = duration.num_days();
        return format!("{days} day{s} ago", s = if days == 1 { "" } else { "s" });
    }
    if duration.num_weeks() < 4 {
        let weeks = duration.num_weeks();
        return format!("{weeks} week{s} ago", s = if weeks == 1 { "" } else { "s" });
    }

    // Fall back to absolute date for older items
    dt.format("%Y-%m-%d").to_string()
}

/// Create a visual score bar using Unicode blocks.
fn score_bar(score: f32) -> String {
    // BM25/TF-IDF scores can vary widely. Higher absolute value = better match
    // The sign might be negated depending on the search implementation
    // Typical range: 5 (weak) to 15+ (strong match) after absolute value
    let abs_score = score.abs();
    let clamped = (abs_score - 5.0).clamp(0.0, 10.0);
    // Safe: clamped is in [0.0, 10.0], so normalized is in [0, 10]
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let normalized = (clamped / 10.0 * 10.0) as usize;
    let filled = normalized.min(10);
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);
    bar
}

/// Style for role badges.
fn role_style(role: &str) -> Style {
    match role.to_lowercase().as_str() {
        "user" => Style::new().cyan().bold(),
        "assistant" => Style::new().green().bold(),
        "system" => Style::new().magenta().bold(),
        "tool" => Style::new().yellow().bold(),
        _ => Style::new().white(),
    }
}

/// Decode HTML entities commonly found in search snippets.
fn decode_html_entities(s: &str) -> String {
    let mut result = s.to_string();

    // Named entities
    result = result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    // Numeric entities (common ones)
    result = result
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x22;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x3c;", "<")
        .replace("&#60;", "<")
        .replace("&#x3e;", ">")
        .replace("&#62;", ">")
        .replace("&#x26;", "&")
        .replace("&#38;", "&");

    result
}

/// Colorize snippet highlights, converting `<b>...</b>` to styled text.
fn colorize_snippet(s: &str) -> String {
    // First decode HTML entities
    let decoded = decode_html_entities(s);

    // Check if we should use colors
    let use_color = console::colors_enabled();

    if !use_color {
        // Strip HTML tags for plain output
        return decoded.replace("<b>", "").replace("</b>", "");
    }

    // Replace <b> tags with ANSI bold yellow, ensuring balanced tags
    // Handle truncated tags at boundaries
    let mut result = decoded
        .replace("<b>", "\x1b[1;33m")
        .replace("</b>", "\x1b[0m");

    // Clean up any unmatched/broken tags that might be truncated
    result = result.replace("<b", "").replace("</b", "\x1b[0m");

    // Ensure we reset at the end in case of truncation
    if result.contains("\x1b[1;33m") && !result.ends_with("\x1b[0m") {
        result.push_str("\x1b[0m");
    }

    result
}

/// Truncate and wrap text to fit terminal width.
fn wrap_text(s: &str, prefix_width: usize, max_lines: usize) -> String {
    let width = term_width().saturating_sub(prefix_width + 2);
    let width = width.max(40); // minimum readable width

    let clean = s.replace('\n', " ").replace('\r', "");
    let wrapped = textwrap::wrap(&clean, width);

    wrapped
        .into_iter()
        .take(max_lines)
        .map(|cow| cow.to_string())
        .collect::<Vec<_>>()
        .join(&format!("\n{:prefix_width$}", ""))
}

/// Print search results in a beautiful format.
pub fn print_search_results(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("{}", style("No results found.").dim());
        return;
    }

    let icons = Icons::detect();
    let width = term_width().min(100); // Cap width for readability
    let separator = "─".repeat(width);
    let double_sep = "═".repeat(width);

    println!("{}", style(&double_sep).dim());
    println!(
        "{}",
        style(format!(" {}Found {} result(s)", icons.search, hits.len()))
            .bold()
            .white()
    );
    println!("{}", style(&double_sep).dim());

    for (i, hit) in hits.iter().enumerate() {
        if i > 0 {
            println!("{}", style(&separator).dim());
        }

        // Line 1: Score bar + Role badge + Source adapter
        let score_display = hit.score.abs(); // Show absolute score (higher = better)
        let bar = score_bar(hit.score);
        let role_str = hit.role.to_string();
        let role_badge = role_style(&role_str).apply_to(&role_str);
        let adapter = style(&hit.source_adapter).cyan();

        print!(" {} ", style(&bar).yellow());
        print!("{} ", style(format!("{score_display:.1}")).dim());
        print!("{role_badge} ");
        println!("{adapter}");

        // Line 2: Title (if present)
        if let Some(title) = &hit.title {
            // Clean up title: remove newlines and excessive whitespace
            let clean_title: String = title
                .chars()
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            let truncated = if clean_title.len() > width - 4 {
                format!("{}...", &clean_title[..width - 7])
            } else {
                clean_title
            };
            println!(
                " {} {}",
                style(icons.bullet).dim(),
                style(truncated).bold()
            );
        }

        // Line 3: Workspace + Host (if present)
        let mut context_parts = Vec::new();
        if let Some(ws) = &hit.workspace {
            // Shorten workspace path for display
            let ws_display = if ws.len() > 50 {
                let parts: Vec<_> = ws.split('/').collect();
                if parts.len() > 3 {
                    format!(".../{}", parts[parts.len() - 2..].join("/"))
                } else {
                    ws.clone()
                }
            } else {
                ws.clone()
            };
            context_parts.push(format!("{}{ws_display}", icons.folder));
        }
        if let Some(host) = &hit.host {
            context_parts.push(format!("{}{host}", icons.host));
        }
        if !context_parts.is_empty() {
            println!(" {}", style(context_parts.join("  ")).dim());
        }

        // Line 4: Dates
        let created = relative_time(hit.conv_created_at);
        let updated = hit
            .conv_updated_at
            .map(|dt| format!(" {} {}", icons.arrow, relative_time(dt)))
            .unwrap_or_default();
        println!(
            " {}",
            style(format!("{}{created}{updated}", icons.calendar))
                .dim()
                .italic()
        );

        // Line 5+: Snippet (wrapped and highlighted)
        let snippet = colorize_snippet(&hit.snippet);
        let wrapped = wrap_text(&snippet, 4, 3);
        println!();
        for line in wrapped.lines() {
            println!("   {line}");
        }
        println!();
    }

    println!("{}", style(&double_sep).dim());
}

/// Print search results in a compact format (for piping/scripting).
#[expect(dead_code)]
pub fn print_search_results_compact(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("No results found.");
        return;
    }

    for hit in hits {
        let score = -hit.score;
        let title = hit.title.as_deref().unwrap_or("-");
        let ws = hit.workspace.as_deref().unwrap_or("-");
        println!(
            "{score:.2}\t{role}\t{adapter}\t{ws}\t{title}",
            role = hit.role,
            adapter = hit.source_adapter,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_time_just_now() {
        let now = Utc::now();
        assert_eq!(relative_time(now), "just now");
    }

    #[test]
    fn test_score_bar() {
        let bar = score_bar(-10.0);
        assert!(bar.contains('█'));
        assert!(bar.contains('░'));
        assert_eq!(bar.chars().count(), 10);
    }
}
