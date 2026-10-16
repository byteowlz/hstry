//! `hstry pick` support: fuzzy scoring, an interactive terminal picker, and
//! transcript formatting for the copy/print actions.

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::io::{IsTerminal, Write};
use uuid::Uuid;

/// A selectable session plus the text the fuzzy query is scored against.
pub struct Candidate {
    pub id: Uuid,
    /// One-line display label.
    pub label: String,
    /// Lowercased `title + first message + workspace + id` for matching.
    pub haystack: String,
}

impl Candidate {
    pub fn new(id: Uuid, label: String, parts: &[&str]) -> Self {
        let haystack = parts
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| part.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            id,
            label,
            haystack,
        }
    }
}

/// Greedy case-insensitive subsequence score. Higher is better; `None` when the
/// query does not appear in order. Consecutive and word-start matches score more
/// so that typing `auth ref` prefers "auth refactor" over a scattered match.
pub fn fuzzy_score(query: &str, haystack: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let mut hay = haystack.chars();
    let mut score = 0i32;
    let mut prev_matched = false;
    let mut any = false;
    let mut last = '\0';

    for qc in query.chars() {
        if qc == ' ' {
            prev_matched = false;
            continue;
        }
        let qc = qc.to_ascii_lowercase();
        let mut matched = false;
        for hc in hay.by_ref() {
            if hc.to_ascii_lowercase() == qc {
                score += 1;
                if prev_matched {
                    score += 4;
                }
                if last.is_whitespace() || matches!(last, '/' | '-' | '_' | '.' | ':') {
                    score += 6;
                }
                prev_matched = true;
                any = true;
                last = hc;
                matched = true;
                break;
            }
            prev_matched = false;
            last = hc;
        }
        if !matched {
            return None;
        }
    }
    Some(if any { score } else { 0 })
}

/// Rank candidate indices by descending fuzzy score (stable). Empty query keeps
/// the incoming order.
pub fn rank(candidates: &[Candidate], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..candidates.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            fuzzy_score(query, &candidate.haystack).map(|score| (score, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

/// Interactive picker. Returns the chosen session, or `None` if cancelled.
/// Only call when stdout is a terminal.
pub fn pick_interactive(candidates: &[Candidate], initial: &str) -> Result<Option<Uuid>> {
    let mut stdout = std::io::stdout();
    terminal::enable_raw_mode().context("enabling raw mode")?;
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let result = pick_loop(&mut stdout, candidates, initial);
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    result
}

fn pick_loop(
    stdout: &mut std::io::Stdout,
    candidates: &[Candidate],
    initial: &str,
) -> Result<Option<Uuid>> {
    let mut filter = initial.to_string();
    let mut selected = 0usize;

    loop {
        let matches = rank(candidates, &filter);
        if selected >= matches.len() {
            selected = matches.len().saturating_sub(1);
        }

        let (width, height) = terminal::size().unwrap_or((100, 30));
        let rows = usize::from(height.saturating_sub(4)).max(1);
        let start = if selected >= rows {
            selected + 1 - rows
        } else {
            0
        };

        let mut frame = String::new();
        frame.push_str(&truncate(&format!("pick> {filter}"), width));
        frame.push('\n');
        frame.push_str(&truncate(
            &format!(
                "{} match(es)   up/down move  Enter select  Esc cancel",
                matches.len()
            ),
            width,
        ));
        frame.push('\n');
        for (row, &index) in matches.iter().enumerate().skip(start).take(rows) {
            let label = truncate(&candidates[index].label, width.saturating_sub(2));
            if row == selected {
                frame.push_str(&format!("> {label}"));
                frame.push_str(&" ".repeat(width as usize));
            } else {
                frame.push_str(&format!("  {label}"));
                frame.push_str(&" ".repeat(width as usize));
            }
            frame.push('\n');
        }

        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
        // Print the frame, then dim/clear the trailing padding on the cursor line.
        let mut lines = frame.lines();
        if let Some(first) = lines.next() {
            queue!(stdout, Print(first))?;
        }
        for line in lines {
            queue!(
                stdout,
                Print("\r\n"),
                SetAttribute(Attribute::Reset),
                Print(line)
            )?;
        }
        let _ = execute!(stdout, SetAttribute(Attribute::Reset));
        stdout.flush()?;

        match event::read()? {
            Event::Key(key)
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
            {
                match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        return Ok(matches.get(selected).map(|&index| candidates[index].id));
                    }
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(matches.len().saturating_sub(1)),
                    KeyCode::PageUp => selected = selected.saturating_sub(rows),
                    KeyCode::PageDown => {
                        selected = (selected + rows).min(matches.len().saturating_sub(1))
                    }
                    KeyCode::Home => selected = 0,
                    KeyCode::End => selected = matches.len().saturating_sub(1),
                    KeyCode::Backspace => {
                        filter.pop();
                        selected = 0;
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        filter.push(ch);
                        selected = 0;
                    }
                    _ => {}
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn truncate(value: &str, width: u16) -> String {
    let flat = value.replace(['\n', '\r', '\t'], " ");
    let max = usize::from(width).max(1);
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// True when an interactive picker can run.
pub fn interactive_available() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Render a conversation transcript as markdown for copy/print.
pub fn format_transcript(
    title: Option<&str>,
    id: Uuid,
    source_id: &str,
    workspace: Option<&str>,
    created_at: chrono::DateTime<chrono::Utc>,
    messages: &[hstry_core::models::Message],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", title.unwrap_or("(untitled)")));
    out.push_str(&format!("- id: `{id}`\n"));
    out.push_str(&format!("- source: `{source_id}`\n"));
    if let Some(workspace) = workspace {
        out.push_str(&format!("- workspace: `{workspace}`\n"));
    }
    out.push_str(&format!("- created: {}\n\n", created_at.to_rfc3339()));
    for message in messages {
        out.push_str(&format!("## {}\n\n", message.role));
        let content = message.content.trim();
        if !content.is_empty() {
            out.push_str(content);
            out.push_str("\n\n");
        }
    }
    out.trim_end().to_string()
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("opening clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("writing clipboard")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_requires_subsequence() {
        assert!(fuzzy_score("auth", "refactor auth middleware").is_some());
        assert!(fuzzy_score("zzz", "refactor auth middleware").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn fuzzy_prefers_consecutive_and_word_starts() {
        let contiguous = fuzzy_score("auth", "auth middleware").unwrap();
        let scattered = fuzzy_score("auth", "axuxthxh").unwrap();
        assert!(
            contiguous > scattered,
            "{contiguous} should beat {scattered}"
        );
    }

    #[test]
    fn rank_orders_by_score_then_stable() {
        let candidates = vec![
            Candidate::new(Uuid::nil(), "billing report".into(), &["billing report"]),
            Candidate::new(
                Uuid::nil(),
                "refactor auth middleware".into(),
                &["refactor auth middleware"],
            ),
            Candidate::new(Uuid::nil(), "auth again".into(), &["auth again"]),
        ];
        let ranked = rank(&candidates, "auth");
        assert_eq!(ranked, vec![2, 1], "best match first, non-matches dropped");
    }

    #[test]
    fn rank_keeps_order_without_query() {
        let candidates = vec![
            Candidate::new(Uuid::nil(), "a".into(), &["a"]),
            Candidate::new(Uuid::nil(), "b".into(), &["b"]),
        ];
        assert_eq!(rank(&candidates, "  "), vec![0, 1]);
    }
}
