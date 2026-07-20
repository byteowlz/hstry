//! Pretty terminal output formatting for hstry CLI.

use chrono::{DateTime, Utc};
use hstry_core::models::SearchHit;
use std::io::{self, IsTerminal};
use std::path::Path;

/// Conversation display data for list output.
#[derive(Debug, Clone)]
pub struct ConversationDisplay {
    pub id: uuid::Uuid,
    pub source_id: String,
    pub workspace: Option<String>,
    pub created_at: DateTime<Utc>,
    pub title: String,
    /// Human-readable id (adjective-noun) when available; shown in the id
    /// column in preference to the UUID prefix.
    pub readable_id: Option<String>,
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

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn display_workspace(workspace: Option<&str>) -> String {
    workspace
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn display_source(source: &str) -> &str {
    source
        .rsplit_once('-')
        .filter(|(_, suffix)| suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_hexdigit()))
        .map_or(source, |(base, _)| base)
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    format!("{}…", value.chars().take(width - 1).collect::<String>())
}

fn print_rows(rows: &[(String, String, String, String, String)], empty: &str) {
    if rows.is_empty() {
        println!("{empty}");
        return;
    }

    if !io::stdout().is_terminal() {
        for (title, workspace, source, age, id) in rows {
            println!("{title}\t{workspace}\t{source}\t{age}\t{id}");
        }
        return;
    }

    let terminal_width = console::Term::stdout().size().1 as usize;
    let workspace_width = rows
        .iter()
        .map(|row| row.1.chars().count())
        .max()
        .unwrap_or(9)
        .clamp(9, 18);
    let source_width = rows
        .iter()
        .map(|row| row.2.chars().count())
        .max()
        .unwrap_or(6)
        .clamp(6, 16);
    let age_width = rows
        .iter()
        .map(|row| row.3.chars().count())
        .max()
        .unwrap_or(3)
        .clamp(3, 10);
    let id_width = rows
        .iter()
        .map(|row| row.4.chars().count())
        .max()
        .unwrap_or(2)
        .clamp(8, 20);
    let fixed_width = workspace_width + source_width + age_width + id_width + 8;
    let title_width = terminal_width.saturating_sub(fixed_width).clamp(24, 72);

    println!(
        "{:<title_width$}  {:<workspace_width$}  {:<source_width$}  {:>age_width$}  {:<id_width$}",
        "TITLE", "WORKSPACE", "SOURCE", "AGE", "ID"
    );
    for (title, workspace, source, age, id) in rows {
        println!(
            "{:<title_width$}  {:<workspace_width$}  {:<source_width$}  {:>age_width$}  {:<id_width$}",
            truncate(title, title_width),
            truncate(workspace, workspace_width),
            truncate(source, source_width),
            age,
            truncate(id, id_width),
        );
    }
}

/// Print search results in a compact format.
pub fn print_search_results(hits: &[SearchHit]) {
    let rows = hits
        .iter()
        .map(|hit| {
            let id = hit
                .readable_id
                .clone()
                .unwrap_or_else(|| hit.conversation_id.to_string()[..8].to_string());
            let title = single_line(hit.title.as_deref().unwrap_or("(untitled)"));
            let snippet = single_line(&clean_snippet(&hit.snippet));
            (
                format!("{title} \u{2014} {snippet}"),
                display_workspace(hit.workspace.as_deref()),
                display_source(&hit.source_adapter).to_string(),
                relative_time_short(hit.conv_created_at),
                id,
            )
        })
        .collect::<Vec<_>>();
    print_rows(&rows, "No results found.");
}

/// Print conversations in a nice table format.
pub fn print_conversations(conversations: &[ConversationDisplay]) {
    let rows = conversations
        .iter()
        .map(|conversation| {
            let id = conversation
                .readable_id
                .clone()
                .unwrap_or_else(|| conversation.id.to_string()[..8].to_string());
            (
                single_line(&conversation.title),
                display_workspace(conversation.workspace.as_deref()),
                display_source(&conversation.source_id).to_string(),
                relative_time_short(conversation.created_at),
                id,
            )
        })
        .collect::<Vec<_>>();
    print_rows(&rows, "No conversations found.");
}

/// Print compact search results (one per session with occurrence count).
pub fn print_search_results_compact(hits: &[SearchHit]) {
    let rows = hits
        .iter()
        .map(|hit| {
            let id = hit
                .readable_id
                .clone()
                .unwrap_or_else(|| hit.conversation_id.to_string()[..8].to_string());
            let title = single_line(hit.title.as_deref().unwrap_or("(untitled)"));
            let snippet = single_line(&clean_snippet(&hit.snippet));
            let occurrences = hit.occurrences.unwrap_or(1);
            let lead = if occurrences > 1 {
                format!("{title} ({occurrences}\u{d7}) \u{2014} {snippet}")
            } else {
                format!("{title} \u{2014} {snippet}")
            };
            (
                lead,
                display_workspace(hit.workspace.as_deref()),
                display_source(&hit.source_adapter).to_string(),
                relative_time_short(hit.conv_created_at),
                id,
            )
        })
        .collect::<Vec<_>>();
    print_rows(&rows, "No results found.");
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
    fn test_clean_snippet() {
        let input = "hello\n\t  world   foo";
        assert_eq!(clean_snippet(input), "hello world foo");
    }

    #[test]
    fn single_line_preserves_words_without_truncation() {
        assert_eq!(
            single_line("Central Server\nfor ROMs"),
            "Central Server for ROMs"
        );
    }
}
