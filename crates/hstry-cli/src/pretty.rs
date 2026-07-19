//! Pretty terminal output formatting for hstry CLI.

use chrono::{DateTime, Utc};
use hstry_core::models::SearchHit;

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

/// Print search results in a compact format.
pub fn print_search_results(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("No results found.");
        return;
    }

    for hit in hits {
        let id = hit
            .readable_id
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| hit.conversation_id.to_string()[..8].to_string());
        let title = single_line(hit.title.as_deref().unwrap_or("(untitled)"));
        let snippet = single_line(&clean_snippet(&hit.snippet));
        let workspace = hit.workspace.as_deref().unwrap_or("-");
        println!(
            "{id}	{}	{}	{}	{workspace}	{title}	{snippet}",
            hit.source_adapter,
            hit.role,
            relative_time_short(hit.conv_created_at),
        );
    }
}

/// Print conversations in a nice table format.
pub fn print_conversations(conversations: &[ConversationDisplay]) {
    if conversations.is_empty() {
        println!("No conversations found.");
        return;
    }

    for conversation in conversations {
        let id = conversation
            .readable_id
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| conversation.id.to_string()[..8].to_string());
        let workspace = conversation.workspace.as_deref().unwrap_or("-");
        let title = single_line(&conversation.title);
        println!(
            "{id}	{}	{}	{workspace}	{title}",
            conversation.source_id,
            relative_time_short(conversation.created_at),
        );
    }
}

/// Print compact search results (one per session with occurrence count).
pub fn print_search_results_compact(hits: &[SearchHit]) {
    if hits.is_empty() {
        println!("No results found.");
        return;
    }

    for hit in hits {
        let id = hit
            .readable_id
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| hit.conversation_id.to_string()[..8].to_string());
        let title = single_line(hit.title.as_deref().unwrap_or("(untitled)"));
        let snippet = single_line(&clean_snippet(&hit.snippet));
        let workspace = hit.workspace.as_deref().unwrap_or("-");
        let occurrences = hit.occurrences.unwrap_or(1);
        println!(
            "{id}	{}	{}	{}	{workspace}	{occurrences}	{title}	{snippet}",
            hit.source_adapter,
            hit.role,
            relative_time_short(hit.conv_created_at),
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
