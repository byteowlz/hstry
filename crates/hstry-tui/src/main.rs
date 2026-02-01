#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::Datelike;
use clap::{Args, Parser};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pulldown_cmark::{Event as MdEvent, Options, Parser as MdParser, Tag, TagEnd};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use uuid::Uuid;

use hstry_core::{
    Config, Database,
    db::ListConversationsOptions,
    models::{Conversation, Message, MessageRole, SearchHit, Source},
};

// =============================================================================
// Markdown Rendering
// =============================================================================

/// Render markdown content to styled ratatui Lines
fn render_markdown(
    content: &str,
    role: &MessageRole,
    highlight: Option<&str>,
) -> Vec<Line<'static>> {
    // For tool output, try special formatting first
    if *role == MessageRole::Tool
        && let Some(lines) = try_format_tool_output(content)
    {
        return lines;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];

    // Track state
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
                        format!("{indent}* "),
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
                    // Store URL for later (simplified: just style the text)
                    let _ = dest_url;
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans);
                    let style = Style::default().fg(Color::DarkGray);
                    style_stack.push(style);
                    current_spans.push(Span::styled("> ", Style::default().fg(Color::DarkGray)));
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
                    lines.push(Line::from("")); // Add blank line after paragraph
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    // Render code block with collapsing for long blocks
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
                    // Collect code block content
                    for line in text.lines() {
                        code_block_lines.push(line.to_string());
                    }
                } else {
                    let style = current_style(&style_stack);
                    // Handle heading prefix
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
                // Inline code
                current_spans.push(Span::styled(
                    format!("`{code}`"),
                    Style::default().fg(Color::Yellow),
                ));
            }
            MdEvent::SoftBreak => {
                if !in_code_block {
                    current_spans.push(Span::raw(" "));
                }
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Rule => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(Line::from("---").fg(Color::DarkGray));
            }
            _ => {}
        }
    }

    // Flush any remaining content
    flush_line(&mut lines, &mut current_spans);

    // Remove trailing empty lines
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
    let collapsed = code_lines.len() > 12;

    if collapsed {
        // Show collapsed view with preview
        lines.push(
            Line::from(format!(
                "[{} block: {} lines]",
                display_lang,
                code_lines.len()
            ))
            .fg(Color::DarkGray),
        );
        for line in code_lines.iter().take(4) {
            lines.push(Line::from(vec![
                Span::styled("  | ", Style::default().fg(Color::DarkGray)),
                Span::styled(truncate_str(line, 70), Style::default().fg(Color::Gray)),
            ]));
        }
        if code_lines.len() > 4 {
            lines.push(Line::from("  | ...").fg(Color::DarkGray));
        }
    } else {
        // Show full code block
        lines.push(Line::from(format!("```{display_lang}")).fg(Color::DarkGray));
        for line in code_lines {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(line.clone(), Style::default().fg(Color::Gray)),
            ]));
        }
        lines.push(Line::from("```").fg(Color::DarkGray));
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(3);
        let truncated: String = s.chars().take(keep).collect();
        format!("{truncated}...")
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
    // Try JSON tool output format
    if let Some(lines) = try_parse_tool_json(content) {
        return Some(lines);
    }

    // Try exit code header format
    if content.starts_with("Exit code:") {
        return Some(format_exit_code_output(content));
    }

    // Try file listing format
    if looks_like_file_listing(content) {
        return Some(format_file_listing(content));
    }

    None
}

fn try_parse_tool_json(content: &str) -> Option<Vec<Line<'static>>> {
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Extract output field
    if let Some(output) = parsed.get("output").and_then(|v| v.as_str()) {
        // Check for success/update messages
        if output.starts_with("Success.") || output.starts_with("Updated") {
            lines.push(Line::from(output.lines().next()?.to_string()).fg(Color::Green));

            // List modified files
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
                        Line::from(format!("  ... and {} more", files.len() - 5))
                            .fg(Color::DarkGray),
                    );
                }
            }
        } else {
            // Regular output - render as markdown or plain text
            let output_lines = render_markdown(output, &MessageRole::Tool, None);
            if output_lines.len() > 25 {
                lines.extend(output_lines.into_iter().take(20));
                lines.push(Line::from("... (truncated)").fg(Color::DarkGray));
            } else {
                lines.extend(output_lines);
            }
        }
    }

    // Add error header if non-zero exit
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

    // Show header for non-zero exit
    if exit_code != 0 {
        let time_str = wall_time.map(|t| format!(" ({t})")).unwrap_or_default();
        lines.push(Line::from(format!("Exit: {exit_code}{time_str}")).fg(Color::Red));
    }

    // Process output
    let output: Vec<&str> = content_lines.iter().skip(output_start).copied().collect();

    if looks_like_file_listing_lines(&output) {
        let total = output.len();
        for f in output.iter().take(8) {
            lines.push(Line::from(format!("  {}", shorten_path(f))).fg(Color::DarkGray));
        }
        if total > 8 {
            lines.push(Line::from(format!("  ... and {} more", total - 8)).fg(Color::DarkGray));
        }
    } else {
        for line in output.iter().take(20) {
            lines.push(Line::from(line.to_string()));
        }
        if output.len() > 20 {
            lines.push(
                Line::from(format!("... ({} more lines)", output.len() - 20)).fg(Color::DarkGray),
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
        lines.push(Line::from(format!("  ... and {remaining} more")).fg(Color::DarkGray));
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

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .common
        .config
        .unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    // Create tokio runtime for async operations
    let rt = tokio::runtime::Runtime::new()?;

    // Open database
    let db = rt.block_on(Database::open(&config.database))?;

    // Load initial data
    let sources = rt.block_on(db.list_sources())?;
    let conversations = rt.block_on(db.list_conversations(ListConversationsOptions {
        limit: None,
        ..Default::default()
    }))?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config, db, sources, conversations);
    let result = run_app(&mut terminal, &mut app, &rt);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Close database
    rt.block_on(app.db.close());

    result
}

#[derive(Debug, Parser)]
#[command(author, version, about = "TUI interface for hstry chat history")]
struct Cli {
    #[command(flatten)]
    common: CommonOpts,
}

#[derive(Debug, Clone, Args)]
struct CommonOpts {
    /// Override the config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

// =============================================================================
// App Mode (Modal System)
// =============================================================================

#[derive(Debug, Clone)]
enum AppMode {
    Normal,
    Search { query: String, cursor: usize },
    Help { scroll: usize },
    Sort,
    Delete { count: usize },
    DeleteSource { source_id: String, source_name: String },
}

impl AppMode {
    fn name(&self) -> &'static str {
        match self {
            AppMode::Normal => "NORMAL",
            AppMode::Search { .. } => "SEARCH",
            AppMode::Help { .. } => "HELP",
            AppMode::Sort => "SORT",
            AppMode::Delete { .. } => "DELETE",
            AppMode::DeleteSource { .. } => "DELETE SOURCE",
        }
    }

    fn color(&self) -> Color {
        match self {
            AppMode::Normal => Color::Green,
            AppMode::Search { .. } => Color::Blue,
            AppMode::Help { .. } => Color::Yellow,
            AppMode::Sort => Color::Magenta,
            AppMode::Delete { .. } | AppMode::DeleteSource { .. } => Color::Red,
        }
    }
}

// =============================================================================
// Focus Pane
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Left,
    Middle,
    Right,
}

// =============================================================================
// Sort Options
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    DateDesc,
    DateAsc,
    TitleAsc,
    TitleDesc,
    SourceAsc,
}

impl SortOrder {
    fn label(self) -> &'static str {
        match self {
            SortOrder::DateDesc => "Date (newest first)",
            SortOrder::DateAsc => "Date (oldest first)",
            SortOrder::TitleAsc => "Title (A-Z)",
            SortOrder::TitleDesc => "Title (Z-A)",
            SortOrder::SourceAsc => "Source",
        }
    }

    fn all() -> &'static [SortOrder] {
        &[
            SortOrder::DateDesc,
            SortOrder::DateAsc,
            SortOrder::TitleAsc,
            SortOrder::TitleDesc,
            SortOrder::SourceAsc,
        ]
    }
}

// =============================================================================
// Search Scope
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchScope {
    Local,
    Remote,
    All,
}

impl SearchScope {
    fn label(self) -> &'static str {
        match self {
            SearchScope::Local => "local",
            SearchScope::Remote => "remote",
            SearchScope::All => "all",
        }
    }

    fn next(self) -> Self {
        match self {
            SearchScope::Local => SearchScope::All,
            SearchScope::All => SearchScope::Remote,
            SearchScope::Remote => SearchScope::Local,
        }
    }
}

// =============================================================================
// Filter State
// =============================================================================

#[derive(Debug, Clone, Default)]
struct FilterState {
    source: Option<String>,
    workspace: Option<String>,
    date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
}

// Import for date filtering
use chrono::{DateTime, NaiveDate, Utc};

// =============================================================================
// Selection State
// =============================================================================

#[derive(Debug, Clone, Default)]
struct Selection {
    index: usize,
    selected_indices: HashSet<usize>,
}

impl Selection {
    fn next(&mut self, max: usize) {
        if max == 0 {
            return;
        }
        if self.index < max - 1 {
            self.index += 1;
        }
    }

    fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        }
    }

    fn top(&mut self) {
        self.index = 0;
    }

    fn bottom(&mut self, max: usize) {
        if max == 0 {
            return;
        }
        self.index = max - 1;
    }

    fn page_down(&mut self, max: usize, page_size: usize) {
        if max == 0 {
            return;
        }
        self.index = (self.index + page_size).min(max - 1);
    }

    fn page_up(&mut self, page_size: usize) {
        self.index = self.index.saturating_sub(page_size);
    }

    fn toggle_selection(&mut self) {
        if self.selected_indices.contains(&self.index) {
            self.selected_indices.remove(&self.index);
        } else {
            self.selected_indices.insert(self.index);
        }
    }

    fn select_all(&mut self, max: usize) {
        self.selected_indices = (0..max).collect();
    }

    fn deselect_all(&mut self) {
        self.selected_indices.clear();
    }

    fn has_selections(&self) -> bool {
        !self.selected_indices.is_empty()
    }
}

// =============================================================================
// Left Pane View Modes
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftPaneView {
    Sources,
    Workspaces,
    Dates,
}

impl LeftPaneView {
    fn label(self) -> &'static str {
        match self {
            Self::Sources => "Sources",
            Self::Workspaces => "Workspaces",
            Self::Dates => "Dates",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Sources => Self::Workspaces,
            Self::Workspaces => Self::Dates,
            Self::Dates => Self::Sources,
        }
    }
}

// =============================================================================
// Navigation Item for Left Pane
// =============================================================================

#[derive(Debug, Clone)]
enum NavItem {
    All,
    Source(String, String), // (id, adapter name)
    Workspace(String),
    // Date grouping items
    DateYear(i32),          // Year (e.g., 2025)
    DateMonth(i32, u32),    // Year, Month (1-12)
    DateDay(i32, u32, u32), // Year, Month, Day
}

impl NavItem {
    fn label(&self) -> String {
        match self {
            NavItem::All => "All Conversations".to_string(),
            NavItem::Source(_, adapter) => adapter.clone(),
            NavItem::Workspace(ws) => format!("@ {ws}"),
            NavItem::DateYear(year) => year.to_string(),
            NavItem::DateMonth(year, month) => {
                let month_names = [
                    "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                    "Nov", "Dec",
                ];
                format!(
                    "{} {}",
                    month_names.get(*month as usize).unwrap_or(&"?"),
                    year
                )
            }
            NavItem::DateDay(year, month, day) => format!("{month:02}/{day:02}/{year}"),
        }
    }
}

// =============================================================================
// App State
// =============================================================================

struct App {
    config: Config,
    db: Database,
    mode: AppMode,
    focus: FocusPane,
    sort_order: SortOrder,
    sort_selection: usize,
    filter: FilterState,
    g_prefix: bool,

    // Data
    sources: Vec<Source>,
    all_conversations: Vec<Conversation>,
    filtered_conversations: Vec<Conversation>,
    messages: Vec<Message>,
    search_results: Vec<SearchHit>,
    show_search_results: bool,
    last_search_query: Option<String>,
    search_scope: SearchScope,

    // Navigation items for left pane
    left_pane_view: LeftPaneView,
    nav_items: Vec<NavItem>,
    nav_selection: Selection,
    // Track expanded date groups
    expanded_dates: HashSet<String>,

    // Middle pane selection
    conv_selection: Selection,

    // Right pane scroll
    detail_scroll: usize,

    // Status message
    status_message: String,
}

impl App {
    fn new(
        config: Config,
        db: Database,
        sources: Vec<Source>,
        conversations: Vec<Conversation>,
    ) -> Self {
        // Build navigation items for Sources view (default)
        let mut nav_items = vec![NavItem::All];
        for source in &sources {
            nav_items.push(NavItem::Source(source.id.clone(), source.adapter.clone()));
        }

        let filtered_conversations = conversations.clone();

        Self {
            config,
            db,
            mode: AppMode::Normal,
            focus: FocusPane::Middle,
            sort_order: SortOrder::DateDesc,
            sort_selection: 0,
            filter: FilterState::default(),
            g_prefix: false,
            sources,
            all_conversations: conversations,
            filtered_conversations,
            messages: Vec::new(),
            search_results: Vec::new(),
            show_search_results: false,
            last_search_query: None,
            search_scope: SearchScope::Local,
            left_pane_view: LeftPaneView::Sources,
            nav_items,
            nav_selection: Selection::default(),
            expanded_dates: HashSet::new(),
            conv_selection: Selection::default(),
            detail_scroll: 0,
            status_message: "Press ? for help, q to quit".to_string(),
        }
    }

    fn active_list_len(&self) -> usize {
        if self.show_search_results && !self.search_results.is_empty() {
            self.search_results.len()
        } else {
            self.filtered_conversations.len()
        }
    }

    fn selected_conversation_id(&self) -> Option<Uuid> {
        if self.show_search_results && !self.search_results.is_empty() {
            self.search_results
                .get(self.conv_selection.index)
                .map(|hit| hit.conversation_id)
        } else {
            self.filtered_conversations
                .get(self.conv_selection.index)
                .map(|conv| conv.id)
        }
    }

    fn selected_conversation(&self) -> Option<&Conversation> {
        let conv_id = self.selected_conversation_id()?;
        self.all_conversations.iter().find(|c| c.id == conv_id)
    }

    fn apply_filters(&mut self) {
        self.filtered_conversations = self
            .all_conversations
            .iter()
            .filter(|c| {
                if let Some(ref source_id) = self.filter.source
                    && &c.source_id != source_id
                {
                    return false;
                }
                if let Some(ref workspace) = self.filter.workspace
                    && c.workspace.as_ref() != Some(workspace)
                {
                    return false;
                }
                if let Some((start, end)) = self.filter.date_range
                    && (c.created_at < start || c.created_at > end)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        self.apply_sort();
        self.conv_selection.index = 0;
        self.conv_selection.deselect_all();
        self.show_search_results = false;
        self.search_results.clear();
        self.last_search_query = None;
    }

    fn apply_sort(&mut self) {
        match self.sort_order {
            SortOrder::DateDesc => {
                self.filtered_conversations
                    .sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
            SortOrder::DateAsc => {
                self.filtered_conversations
                    .sort_by(|a, b| a.created_at.cmp(&b.created_at));
            }
            SortOrder::TitleAsc => {
                self.filtered_conversations.sort_by(|a, b| {
                    a.title
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.title.as_deref().unwrap_or(""))
                });
            }
            SortOrder::TitleDesc => {
                self.filtered_conversations.sort_by(|a, b| {
                    b.title
                        .as_deref()
                        .unwrap_or("")
                        .cmp(a.title.as_deref().unwrap_or(""))
                });
            }
            SortOrder::SourceAsc => {
                self.filtered_conversations
                    .sort_by(|a, b| a.source_id.cmp(&b.source_id));
            }
        }
    }

    fn toggle_date_expand(&mut self, key: &str) {
        if self.expanded_dates.contains(key) {
            self.expanded_dates.remove(key);
        } else {
            self.expanded_dates.insert(key.to_string());
        }
    }

    fn rebuild_nav_items(&mut self) {
        self.nav_items.clear();

        match self.left_pane_view {
            LeftPaneView::Sources => {
                self.nav_items.push(NavItem::All);
                for source in &self.sources {
                    self.nav_items
                        .push(NavItem::Source(source.id.clone(), source.adapter.clone()));
                }
            }
            LeftPaneView::Workspaces => {
                self.nav_items.push(NavItem::All);
                // Collect unique workspaces
                let mut workspaces: HashSet<String> = HashSet::new();
                for conv in &self.all_conversations {
                    if let Some(ws) = &conv.workspace {
                        workspaces.insert(ws.clone());
                    }
                }
                let mut ws_vec: Vec<_> = workspaces.into_iter().collect();
                ws_vec.sort();
                for ws in ws_vec {
                    self.nav_items.push(NavItem::Workspace(ws));
                }
            }
            LeftPaneView::Dates => {
                // Build date hierarchy
                // Collect all years from conversations
                let mut years: HashSet<i32> = HashSet::new();
                for conv in &self.all_conversations {
                    years.insert(conv.created_at.year());
                }
                let mut years: Vec<i32> = years.into_iter().collect();
                years.sort_by(|a, b| b.cmp(a)); // Descending (newest first)

                for year in years {
                    self.nav_items.push(NavItem::DateYear(year));

                    // If year is expanded, add months
                    if self.expanded_dates.contains(&format!("year:{year}")) {
                        let mut months: HashSet<u32> = HashSet::new();
                        for conv in &self.all_conversations {
                            if conv.created_at.year() == year {
                                months.insert(conv.created_at.month());
                            }
                        }
                        let mut months: Vec<u32> = months.into_iter().collect();
                        months.sort_by(|a, b| b.cmp(a)); // Descending

                        for month in months {
                            self.nav_items.push(NavItem::DateMonth(year, month));

                            // If month is expanded, add days
                            if self
                                .expanded_dates
                                .contains(&format!("month:{year}-{month}"))
                            {
                                let mut days: HashSet<u32> = HashSet::new();
                                for conv in &self.all_conversations {
                                    if conv.created_at.year() == year
                                        && conv.created_at.month() == month
                                    {
                                        days.insert(conv.created_at.day());
                                    }
                                }
                                let mut days: Vec<u32> = days.into_iter().collect();
                                days.sort_by(|a, b| b.cmp(a)); // Descending

                                for day in days {
                                    self.nav_items.push(NavItem::DateDay(year, month, day));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Reset selection if it's now out of bounds
        if self.nav_selection.index >= self.nav_items.len() && !self.nav_items.is_empty() {
            self.nav_selection.index = self.nav_items.len() - 1;
        }
    }

    fn cycle_left_pane_view(&mut self) {
        self.left_pane_view = self.left_pane_view.next();
        self.rebuild_nav_items();
        self.nav_selection.index = 0;
        self.status_message = format!("View: {}", self.left_pane_view.label());
    }

    fn load_messages(&mut self, rt: &tokio::runtime::Runtime) {
        if self.show_search_results
            && let Some(hit) = self.search_results.get(self.conv_selection.index)
            && let Some(host) = &hit.host
        {
            if let Some(remote) = self.config.remotes.iter().find(|r| r.name == *host) {
                match rt.block_on(hstry_core::remote::show_remote(
                    remote,
                    &hit.conversation_id.to_string(),
                )) {
                    Ok(details) => {
                        self.messages = details.messages.into_iter().map(|m| m.message).collect();
                        self.detail_scroll = 0;
                        return;
                    }
                    Err(e) => {
                        self.status_message = format!("Remote load error: {e}");
                        self.messages.clear();
                        return;
                    }
                }
            }
            self.status_message = format!("Remote '{host}' not found in config");
            self.messages.clear();
            return;
        }

        if let Some(conv_id) = self.selected_conversation_id() {
            match rt.block_on(self.db.get_messages(conv_id)) {
                Ok(msgs) => {
                    self.messages = msgs;
                    self.detail_scroll = self
                        .last_search_query
                        .as_deref()
                        .and_then(|q| first_match_scroll(&self.messages, q))
                        .unwrap_or(0);
                }
                Err(e) => {
                    self.status_message = format!("Error loading messages: {e}");
                }
            }
        } else {
            self.messages.clear();
        }
    }

    fn perform_search(&mut self, rt: &tokio::runtime::Runtime) {
        if let AppMode::Search { ref query, .. } = self.mode {
            if query.is_empty() {
                self.search_results.clear();
                self.show_search_results = false;
                self.last_search_query = None;
                return;
            }

            let opts = hstry_core::db::SearchOptions {
                limit: Some(100),
                source_id: self.filter.source.clone(),
                workspace: self.filter.workspace.clone(),
                ..Default::default()
            };
            let search_scope = self.search_scope;
            let config = &self.config;
            let db = &self.db;
            let query = query.clone();
            let query_for_closure = query.clone();

            let search = async move {
                let query = query_for_closure;
                let mut results = Vec::new();
                if search_scope != SearchScope::Remote {
                    let local = if let Some(hits) =
                        hstry_core::service::try_service_search(&query, &opts).await?
                    {
                        hits
                    } else {
                        let index_path = config.search_index_path();
                        hstry_core::search_tantivy::search_with_fallback(
                            db,
                            &index_path,
                            &query,
                            &opts,
                        )
                        .await?
                    };
                    results.extend(local);
                }

                if search_scope != SearchScope::Local {
                    let remote_hits =
                        hstry_core::remote::search_remotes(&config.remotes, &query, &opts).await?;
                    results.extend(remote_hits);
                }

                results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok::<_, hstry_core::Error>(results)
            };

            match rt.block_on(search) {
                Ok(results) => {
                    self.search_results = results;
                    self.show_search_results = !self.search_results.is_empty();
                    self.last_search_query = Some(query.clone());
                    self.conv_selection.index = 0;
                    self.status_message = format!(
                        "Found {} results ({})",
                        self.search_results.len(),
                        self.search_scope.label()
                    );
                }
                Err(e) => {
                    self.status_message = format!("Search error: {e}");
                    self.search_results.clear();
                    self.show_search_results = false;
                    self.last_search_query = None;
                }
            }
        }
    }

    fn refresh_data(&mut self, rt: &tokio::runtime::Runtime) {
        match rt.block_on(self.db.list_sources()) {
            Ok(sources) => self.sources = sources,
            Err(e) => self.status_message = format!("Error loading sources: {e}"),
        }

        match rt.block_on(self.db.list_conversations(ListConversationsOptions {
            limit: None,
            ..Default::default()
        })) {
            Ok(convs) => {
                self.all_conversations = convs;
                self.apply_filters();
                self.show_search_results = false;
                self.search_results.clear();
                self.last_search_query = None;
                self.status_message = "Data refreshed".to_string();
            }
            Err(e) => self.status_message = format!("Error loading conversations: {e}"),
        }
    }
}

// =============================================================================
// Key Action
// =============================================================================

#[derive(Debug, Clone, Copy)]
enum KeyAction {
    Quit,
    Up,
    Down,
    Left,
    Right,
    PageDown,
    PageUp,
    Home,
    End,
    Select,
    Escape,
    Backspace,
    Delete,
    Char(char),
    ToggleSelect,
    SelectAll,
    Tab,
    Noop,
}

fn parse_key(key: &event::KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => KeyAction::Quit,
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => KeyAction::PageDown,
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => KeyAction::PageUp,
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => KeyAction::SelectAll,
        KeyCode::Up => KeyAction::Up,
        KeyCode::Down => KeyAction::Down,
        KeyCode::Left => KeyAction::Left,
        KeyCode::Right => KeyAction::Right,
        KeyCode::PageDown => KeyAction::PageDown,
        KeyCode::PageUp => KeyAction::PageUp,
        KeyCode::Home => KeyAction::Home,
        KeyCode::End => KeyAction::End,
        KeyCode::Enter => KeyAction::Select,
        KeyCode::Esc => KeyAction::Escape,
        KeyCode::Backspace => KeyAction::Backspace,
        KeyCode::Delete => KeyAction::Delete,
        KeyCode::Tab => KeyAction::Tab,
        KeyCode::Char(' ') => KeyAction::ToggleSelect,
        KeyCode::Char(c) => KeyAction::Char(c),
        _ => KeyAction::Noop,
    }
}

// =============================================================================
// Event Loop
// =============================================================================

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    // Load initial messages if we have conversations
    if !app.filtered_conversations.is_empty() {
        app.load_messages(rt);
    }

    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let action = parse_key(&key);

            // Handle Ctrl-C globally
            if matches!(action, KeyAction::Quit) {
                return Ok(());
            }

            match &app.mode {
                AppMode::Normal => {
                    if handle_normal_mode(app, action, rt) {
                        return Ok(());
                    }
                }
                AppMode::Search { .. } => {
                    handle_search_mode(app, action, rt);
                }
                AppMode::Help { .. } => {
                    handle_help_mode(app, action);
                }
                AppMode::Sort => {
                    handle_sort_mode(app, action);
                }
                AppMode::Delete { .. } => {
                    handle_delete_mode(app, action, rt);
                }
                AppMode::DeleteSource { .. } => {
                    handle_delete_source_mode(app, action, rt);
                }
            }
        }
    }
}

fn handle_normal_mode(app: &mut App, action: KeyAction, rt: &tokio::runtime::Runtime) -> bool {
    // Reset g_prefix on any non-g key
    let was_g_prefix = app.g_prefix;
    if !matches!(action, KeyAction::Char('g')) {
        app.g_prefix = false;
    }

    match action {
        KeyAction::Char('q') => return true,
        KeyAction::Char('?') => {
            app.mode = AppMode::Help { scroll: 0 };
        }
        KeyAction::Char('/' | ':') => {
            app.mode = AppMode::Search {
                query: String::new(),
                cursor: 0,
            };
            app.search_results.clear();
            app.show_search_results = false;
        }
        KeyAction::Char('x') => {
            if app.show_search_results {
                app.show_search_results = false;
                app.search_results.clear();
                app.last_search_query = None;
                app.conv_selection.index = 0;
                app.status_message = "Cleared search results".to_string();
            }
        }
        KeyAction::Char('s') => {
            app.mode = AppMode::Sort;
            app.sort_selection = SortOrder::all()
                .iter()
                .position(|&s| s == app.sort_order)
                .unwrap_or(0);
        }
        KeyAction::Char('d') => {
            // Delete conversations when in middle pane, or source when in left pane with source selected
            if app.focus == FocusPane::Left {
                if let Some(NavItem::Source(id, name)) = app.nav_items.get(app.nav_selection.index) {
                    app.mode = AppMode::DeleteSource {
                        source_id: id.clone(),
                        source_name: name.clone(),
                    };
                }
            } else {
                let count = if app.conv_selection.has_selections() {
                    app.conv_selection.selected_indices.len()
                } else {
                    usize::from(!app.filtered_conversations.is_empty())
                };
                if count > 0 {
                    app.mode = AppMode::Delete { count };
                }
            }
        }
        KeyAction::Char('r') => {
            app.refresh_data(rt);
        }
        KeyAction::Char('j') | KeyAction::Down => {
            handle_navigation(app, NavDirection::Down, rt);
        }
        KeyAction::Char('k') | KeyAction::Up => {
            handle_navigation(app, NavDirection::Up, rt);
        }
        KeyAction::Char('h') | KeyAction::Left => match app.focus {
            FocusPane::Middle => app.focus = FocusPane::Left,
            FocusPane::Right => app.focus = FocusPane::Middle,
            FocusPane::Left => {}
        },
        KeyAction::Char('l') | KeyAction::Right => match app.focus {
            FocusPane::Left => app.focus = FocusPane::Middle,
            FocusPane::Middle => app.focus = FocusPane::Right,
            FocusPane::Right => {}
        },
        KeyAction::Char('g') => {
            if was_g_prefix {
                handle_navigation(app, NavDirection::Top, rt);
                app.g_prefix = false;
            } else {
                app.g_prefix = true;
            }
        }
        KeyAction::Char('G') | KeyAction::End => {
            handle_navigation(app, NavDirection::Bottom, rt);
        }
        KeyAction::Home => {
            handle_navigation(app, NavDirection::Top, rt);
        }
        KeyAction::PageDown => {
            handle_navigation(app, NavDirection::PageDown, rt);
        }
        KeyAction::PageUp => {
            handle_navigation(app, NavDirection::PageUp, rt);
        }
        KeyAction::ToggleSelect => {
            if app.focus == FocusPane::Middle {
                app.conv_selection.toggle_selection();
                app.conv_selection.next(app.active_list_len());
            }
        }
        KeyAction::SelectAll => {
            if app.focus == FocusPane::Middle {
                app.conv_selection.select_all(app.active_list_len());
            }
        }
        KeyAction::Char('V') => {
            app.conv_selection.deselect_all();
        }
        KeyAction::Tab => {
            // Cycle left pane view when focused on left pane
            if app.focus == FocusPane::Left {
                app.cycle_left_pane_view();
            }
        }
        KeyAction::Select => {
            if app.focus == FocusPane::Left {
                // Handle selection based on nav item type
                if let Some(nav_item) = app.nav_items.get(app.nav_selection.index) {
                    match nav_item {
                        NavItem::All => {
                            app.filter.source = None;
                            app.filter.workspace = None;
                            app.filter.date_range = None;
                        }
                        NavItem::Source(id, _) => {
                            app.filter.source = Some(id.clone());
                            app.filter.workspace = None;
                            app.filter.date_range = None;
                        }
                        NavItem::Workspace(ws) => {
                            app.filter.source = None;
                            app.filter.workspace = Some(ws.clone());
                            app.filter.date_range = None;
                        }
                        NavItem::DateYear(year) => {
                            app.toggle_date_expand(&format!("year:{year}"));
                            app.rebuild_nav_items();
                            return false;
                        }
                        NavItem::DateMonth(year, month) => {
                            app.toggle_date_expand(&format!("month:{year}-{month}"));
                            app.rebuild_nav_items();
                            return false;
                        }
                        NavItem::DateDay(year, month, day) => {
                            // Filter to specific day
                            let start = NaiveDate::from_ymd_opt(*year, *month, *day)
                                .and_then(|d| d.and_hms_opt(0, 0, 0))
                                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
                            let end = NaiveDate::from_ymd_opt(*year, *month, *day)
                                .and_then(|d| d.and_hms_opt(23, 59, 59))
                                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
                            app.filter.date_range = start.zip(end);
                            app.filter.source = None;
                            app.filter.workspace = None;
                        }
                    }
                    app.apply_filters();
                    app.focus = FocusPane::Middle;
                    if !app.filtered_conversations.is_empty() {
                        app.load_messages(rt);
                    }
                }
            } else if app.focus == FocusPane::Middle {
                // Load messages for selected conversation
                app.load_messages(rt);
                app.focus = FocusPane::Right;
            }
        }
        _ => {}
    }
    false
}

#[derive(Debug, Clone, Copy)]
enum NavDirection {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

fn handle_navigation(app: &mut App, direction: NavDirection, rt: &tokio::runtime::Runtime) {
    let page_size = 20; // Approximate visible items

    match app.focus {
        FocusPane::Left => {
            let max = app.nav_items.len();
            match direction {
                NavDirection::Up => app.nav_selection.previous(),
                NavDirection::Down => app.nav_selection.next(max),
                NavDirection::Top => app.nav_selection.top(),
                NavDirection::Bottom => app.nav_selection.bottom(max),
                NavDirection::PageUp => app.nav_selection.page_up(page_size),
                NavDirection::PageDown => app.nav_selection.page_down(max, page_size),
            }
        }
        FocusPane::Middle => {
            let max = app.active_list_len();
            if max == 0 {
                return;
            }
            let prev_index = app.conv_selection.index;
            match direction {
                NavDirection::Up => app.conv_selection.previous(),
                NavDirection::Down => app.conv_selection.next(max),
                NavDirection::Top => app.conv_selection.top(),
                NavDirection::Bottom => app.conv_selection.bottom(max),
                NavDirection::PageUp => app.conv_selection.page_up(page_size),
                NavDirection::PageDown => app.conv_selection.page_down(max, page_size),
            }
            // Load messages if selection changed
            if app.conv_selection.index != prev_index {
                app.load_messages(rt);
            }
        }
        FocusPane::Right => match direction {
            NavDirection::Up => {
                app.detail_scroll = app.detail_scroll.saturating_sub(1);
            }
            NavDirection::Down => {
                app.detail_scroll += 1;
            }
            NavDirection::Top => {
                app.detail_scroll = 0;
            }
            NavDirection::Bottom => {
                app.detail_scroll = app.messages.len().saturating_sub(1);
            }
            NavDirection::PageUp => {
                app.detail_scroll = app.detail_scroll.saturating_sub(page_size);
            }
            NavDirection::PageDown => {
                app.detail_scroll += page_size;
            }
        },
    }
}

fn handle_search_mode(app: &mut App, action: KeyAction, rt: &tokio::runtime::Runtime) {
    if let AppMode::Search {
        ref mut query,
        ref mut cursor,
    } = app.mode
    {
        match action {
            KeyAction::Escape => {
                app.mode = AppMode::Normal;
            }
            KeyAction::Select => {
                app.perform_search(rt);
            }
            KeyAction::Backspace => {
                if *cursor > 0 {
                    query.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
            KeyAction::Delete => {
                if *cursor < query.len() {
                    query.remove(*cursor);
                }
            }
            KeyAction::Left => {
                *cursor = cursor.saturating_sub(1);
            }
            KeyAction::Right => {
                *cursor = (*cursor + 1).min(query.len());
            }
            KeyAction::Home => {
                *cursor = 0;
            }
            KeyAction::End => {
                *cursor = query.len();
            }
            KeyAction::Char(c) => {
                query.insert(*cursor, c);
                *cursor += 1;
            }
            KeyAction::Tab => {
                app.search_scope = app.search_scope.next();
                app.status_message = format!("Search scope: {}", app.search_scope.label());
            }
            KeyAction::Down => {
                // Navigate search results
                let max = app.search_results.len();
                app.conv_selection.next(max);
            }
            KeyAction::Up => {
                app.conv_selection.previous();
            }
            _ => {}
        }
    }
}

fn handle_help_mode(app: &mut App, action: KeyAction) {
    if let AppMode::Help { ref mut scroll } = app.mode {
        match action {
            KeyAction::Escape | KeyAction::Char('q' | '?') => {
                app.mode = AppMode::Normal;
            }
            KeyAction::Down | KeyAction::Char('j') => {
                *scroll += 1;
            }
            KeyAction::Up | KeyAction::Char('k') => {
                *scroll = scroll.saturating_sub(1);
            }
            KeyAction::PageDown => {
                *scroll += 10;
            }
            KeyAction::PageUp => {
                *scroll = scroll.saturating_sub(10);
            }
            KeyAction::Home | KeyAction::Char('g') => {
                *scroll = 0;
            }
            KeyAction::End | KeyAction::Char('G') => {
                *scroll = 100; // Will be clamped
            }
            _ => {}
        }
    }
}

fn handle_sort_mode(app: &mut App, action: KeyAction) {
    match action {
        KeyAction::Escape => {
            app.mode = AppMode::Normal;
        }
        KeyAction::Down | KeyAction::Char('j') => {
            let max = SortOrder::all().len();
            app.sort_selection = (app.sort_selection + 1) % max;
        }
        KeyAction::Up | KeyAction::Char('k') => {
            let max = SortOrder::all().len();
            app.sort_selection = app.sort_selection.checked_sub(1).unwrap_or(max - 1);
        }
        KeyAction::Select => {
            app.sort_order = SortOrder::all()[app.sort_selection];
            app.apply_sort();
            app.mode = AppMode::Normal;
            app.status_message = format!("Sorted by: {}", app.sort_order.label());
        }
        KeyAction::Char(c) if c.is_ascii_digit() => {
            if let Some(idx) = c.to_digit(10) {
                let idx = idx as usize;
                if idx > 0 && idx <= SortOrder::all().len() {
                    app.sort_order = SortOrder::all()[idx - 1];
                    app.apply_sort();
                    app.mode = AppMode::Normal;
                    app.status_message = format!("Sorted by: {}", app.sort_order.label());
                }
            }
        }
        _ => {}
    }
}

fn handle_delete_mode(app: &mut App, action: KeyAction, rt: &tokio::runtime::Runtime) {
    match action {
        KeyAction::Escape | KeyAction::Char('n') => {
            app.mode = AppMode::Normal;
        }
        KeyAction::Char('y') => {
            // Get conversations to delete
            let to_delete: Vec<uuid::Uuid> = if app.conv_selection.has_selections() {
                app.conv_selection
                    .selected_indices
                    .iter()
                    .filter_map(|&idx| {
                        if app.show_search_results {
                            app.search_results.get(idx).map(|h| h.conversation_id)
                        } else {
                            app.filtered_conversations.get(idx).map(|c| c.id)
                        }
                    })
                    .collect()
            } else {
                // Delete current selection
                let idx = app.conv_selection.index;
                if app.show_search_results {
                    app.search_results
                        .get(idx)
                        .map(|h| vec![h.conversation_id])
                        .unwrap_or_default()
                } else {
                    app.filtered_conversations
                        .get(idx)
                        .map(|c| vec![c.id])
                        .unwrap_or_default()
                }
            };

            let count = to_delete.len();
            let mut deleted = 0;
            for id in to_delete {
                if rt.block_on(app.db.delete_conversation(id)).is_ok() {
                    deleted += 1;
                }
            }

            app.status_message = format!("Deleted {deleted}/{count} conversations");
            app.mode = AppMode::Normal;
            app.conv_selection.deselect_all();
            app.conv_selection.index = 0;
            app.refresh_data(rt);
        }
        _ => {}
    }
}

fn handle_delete_source_mode(app: &mut App, action: KeyAction, rt: &tokio::runtime::Runtime) {
    let source_id = if let AppMode::DeleteSource { source_id, .. } = &app.mode {
        source_id.clone()
    } else {
        return;
    };

    match action {
        KeyAction::Escape | KeyAction::Char('n') => {
            app.mode = AppMode::Normal;
        }
        KeyAction::Char('y') => {
            match rt.block_on(app.db.remove_source(&source_id)) {
                Ok(()) => {
                    app.status_message = format!("Deleted source '{source_id}'");
                    app.nav_selection.index = 0;
                    app.filter.source = None;
                }
                Err(e) => {
                    app.status_message = format!("Error deleting source: {e}");
                }
            }
            app.mode = AppMode::Normal;
            app.refresh_data(rt);
        }
        _ => {}
    }
}

// =============================================================================
// UI Rendering
// =============================================================================

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(chunks[0]);

    draw_left_pane(f, app, main_chunks[0]);
    draw_middle_pane(f, app, main_chunks[1]);
    draw_right_pane(f, app, main_chunks[2]);
    draw_status_bar(f, app, chunks[1]);

    // Draw modal overlays
    match &app.mode {
        AppMode::Help { scroll } => draw_help_overlay(f, *scroll),
        AppMode::Sort => draw_sort_overlay(f, app),
        AppMode::Search { query, cursor } => {
            draw_search_overlay(f, query, *cursor, app.search_scope);
        }
        AppMode::Delete { count } => draw_delete_overlay(f, *count),
        AppMode::DeleteSource { source_name, .. } => draw_delete_source_overlay(f, source_name),
        AppMode::Normal => {}
    }
}

fn draw_left_pane(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == FocusPane::Left;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let bg = Color::Black;
    let base_style = Style::default().bg(bg);

    // Paint a solid background so the three columns feel distinct.
    f.render_widget(Paragraph::new("").style(base_style), area);

    let block = Block::default()
        .title(format!(" {} ", app.left_pane_view.label()))
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(base_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .nav_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == app.nav_selection.index;
            let is_active = match item {
                NavItem::All => app.filter.source.is_none() && app.filter.workspace.is_none(),
                NavItem::Source(id, _) => app.filter.source.as_ref() == Some(id),
                NavItem::Workspace(ws) => app.filter.workspace.as_ref() == Some(ws),
                NavItem::DateYear(_) | NavItem::DateMonth(_, _) | NavItem::DateDay(_, _, _) => {
                    false
                }
            };

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let prefix = match item {
                NavItem::All => " * ",
                NavItem::Source(_, _) | NavItem::Workspace(_) => "   ",
                NavItem::DateYear(year) => {
                    let key = format!("year:{year}");
                    if app.expanded_dates.contains(&key) {
                        "[-] "
                    } else {
                        "[+] "
                    }
                }
                NavItem::DateMonth(year, month) => {
                    let key = format!("month:{year}-{month}");
                    if app.expanded_dates.contains(&key) {
                        "[-] "
                    } else {
                        "[+] "
                    }
                }
                NavItem::DateDay(_, _, _) => "     ",
            };

            ListItem::new(format!("{}{}", prefix, item.label())).style(style)
        })
        .collect();

    let list = List::new(items).highlight_symbol("> ").style(base_style);
    let mut state = ListState::default().with_selected(Some(app.nav_selection.index));
    f.render_stateful_widget(list, inner, &mut state);
}

fn draw_middle_pane(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == FocusPane::Middle;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let bg = Color::Black;
    let base_style = Style::default().bg(bg);

    f.render_widget(Paragraph::new("").style(base_style), area);

    let title = if app.show_search_results && !app.search_results.is_empty() {
        format!(" Search Results ({}) ", app.search_results.len())
    } else {
        format!(" Conversations ({}) ", app.filtered_conversations.len())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(base_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render either search results or conversations
    if app.show_search_results && !app.search_results.is_empty() {
        let items: Vec<ListItem> = app
            .search_results
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let is_selected = i == app.conv_selection.index;
                let style = if is_selected && is_focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let title = hit.title.as_deref().unwrap_or("Untitled");
                let snippet = &hit.snippet;
                let host = hit.host.as_deref().unwrap_or("local");
                let source = &hit.source_adapter;
                ListItem::new(vec![
                    Line::from(title).style(style),
                    Line::from(format!("    {source} | {host} | {snippet}")).fg(Color::DarkGray),
                ])
            })
            .collect();

        let list = List::new(items).highlight_symbol("> ").style(base_style);
        let mut state = ListState::default().with_selected(Some(app.conv_selection.index));
        f.render_stateful_widget(list, inner, &mut state);
    } else {
        let items: Vec<ListItem> = app
            .filtered_conversations
            .iter()
            .enumerate()
            .map(|(i, conv)| {
                let is_selected = i == app.conv_selection.index;
                let is_multi_selected = app.conv_selection.selected_indices.contains(&i);

                let style = if is_selected && is_focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if is_multi_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };

                let marker = if is_multi_selected { "[x] " } else { "    " };
                let title = conv.title.as_deref().unwrap_or("Untitled");
                let date = conv.created_at.format("%Y-%m-%d");
                let source = &conv.source_id;

                ListItem::new(vec![
                    Line::from(format!("{marker}{title}")).style(style),
                    Line::from(format!("      {date} | {source}")).fg(Color::DarkGray),
                ])
            })
            .collect();

        let list = List::new(items).highlight_symbol("> ").style(base_style);
        let mut state = ListState::default().with_selected(Some(app.conv_selection.index));
        f.render_stateful_widget(list, inner, &mut state);
    }
}

fn build_message_lines(messages: &[Message], highlight: Option<&str>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for msg in messages {
        let role_style = match msg.role {
            MessageRole::User => Style::default().fg(Color::Green).bold(),
            MessageRole::Assistant => Style::default().fg(Color::Blue).bold(),
            MessageRole::System => Style::default().fg(Color::Yellow).bold(),
            MessageRole::Tool => Style::default().fg(Color::Magenta).bold(),
            MessageRole::Other => Style::default().fg(Color::Gray).bold(),
        };

        let role_label = match msg.role {
            MessageRole::User => "USER",
            MessageRole::Assistant => "ASSISTANT",
            MessageRole::System => "SYSTEM",
            MessageRole::Tool => "TOOL",
            MessageRole::Other => "OTHER",
        };

        lines.push(Line::from(Span::styled(
            format!("[{role_label}]"),
            role_style,
        )));

        let content_lines = render_markdown(&msg.content, &msg.role, highlight);
        lines.extend(content_lines);

        lines.push(Line::from(""));
    }
    lines
}

fn first_match_scroll(messages: &[Message], query: &str) -> Option<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let lines = build_message_lines(messages, None);
    for (idx, line) in lines.iter().enumerate() {
        if line
            .spans
            .iter()
            .any(|span| span.content.as_ref().to_lowercase().contains(&needle))
        {
            return Some(idx.saturating_sub(1));
        }
    }
    None
}

fn draw_right_pane(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == FocusPane::Right;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let bg = Color::Black;
    let base_style = Style::default().bg(bg);

    f.render_widget(Paragraph::new("").style(base_style), area);

    let block = Block::default()
        .title(" Messages ")
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(base_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.messages.is_empty() {
        if let Some(conv) = app.selected_conversation() {
            let info = vec![
                Line::from(conv.title.as_deref().unwrap_or("Untitled")).bold(),
                Line::from(""),
                Line::from(format!("Source: {}", conv.source_id)),
                Line::from(format!(
                    "Created: {}",
                    conv.created_at.format("%Y-%m-%d %H:%M")
                )),
                if let Some(ws) = &conv.workspace {
                    Line::from(format!("Workspace: {ws}"))
                } else {
                    Line::from("")
                },
                if let Some(model) = &conv.model {
                    Line::from(format!("Model: {model}"))
                } else {
                    Line::from("")
                },
                Line::from(""),
                Line::from("No messages loaded. Press Enter to load.").fg(Color::DarkGray),
            ];
            let paragraph = Paragraph::new(info).style(base_style);
            f.render_widget(paragraph, inner);
        } else {
            let paragraph = Paragraph::new("No conversation selected")
                .fg(Color::DarkGray)
                .style(base_style);
            f.render_widget(paragraph, inner);
        }
        return;
    }

    let highlight = if app.show_search_results {
        app.last_search_query.as_deref()
    } else {
        None
    };
    let lines = build_message_lines(&app.messages, highlight);

    // Apply scroll offset
    let scroll_offset = app.detail_scroll.min(lines.len().saturating_sub(1));

    let scroll = u16::try_from(scroll_offset).unwrap_or(u16::MAX);
    let paragraph = Paragraph::new(lines)
        .style(base_style)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(paragraph, inner);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let mode_style = Style::default().fg(Color::Black).bg(app.mode.color());

    let focus_info = match app.focus {
        FocusPane::Left => "Sources",
        FocusPane::Middle => "Conversations",
        FocusPane::Right => "Messages",
    };

    let selection_info = if app.conv_selection.has_selections() {
        format!(" | {} selected", app.conv_selection.selected_indices.len())
    } else {
        String::new()
    };

    let g_prefix_indicator = if app.g_prefix { " g-" } else { "" };

    let status = Line::from(vec![
        Span::styled(format!(" {} ", app.mode.name()), mode_style),
        Span::raw(" "),
        Span::raw(focus_info),
        Span::raw(selection_info),
        Span::styled(g_prefix_indicator, Style::default().fg(Color::Yellow)),
        Span::raw(" | "),
        Span::raw(&app.status_message),
    ]);

    f.render_widget(Paragraph::new(status), area);
}

fn draw_help_overlay(f: &mut Frame, scroll: usize) {
    let area = centered_rect(70, 80, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help (q/Esc to close) ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let help_text = vec![
        Line::from("NAVIGATION").bold(),
        Line::from(""),
        Line::from("  j / Down      Move down"),
        Line::from("  k / Up        Move up"),
        Line::from("  h / Left      Focus left pane"),
        Line::from("  l / Right     Focus right pane"),
        Line::from("  gg            Jump to top"),
        Line::from("  G             Jump to bottom"),
        Line::from("  Ctrl-d        Page down"),
        Line::from("  Ctrl-u        Page up"),
        Line::from("  Enter         Select/expand"),
        Line::from(""),
        Line::from("LEFT PANE").bold(),
        Line::from(""),
        Line::from("  Tab           Cycle view (Sources/Workspaces/Dates)"),
        Line::from("  Enter         Expand/collapse or filter"),
        Line::from(""),
        Line::from("SELECTION").bold(),
        Line::from(""),
        Line::from("  Space         Toggle selection + move down"),
        Line::from("  Ctrl-a        Select all"),
        Line::from("  V             Clear selection"),
        Line::from(""),
        Line::from("ACTIONS").bold(),
        Line::from(""),
        Line::from("  /             Search"),
        Line::from("  s             Sort options"),
        Line::from("  d             Delete selected"),
        Line::from("  r             Refresh data"),
        Line::from("  ?             Toggle help"),
        Line::from("  q             Quit"),
        Line::from(""),
        Line::from("SEARCH MODE").bold(),
        Line::from(""),
        Line::from("  Enter         Execute search"),
        Line::from("  Esc           Exit search input (keep results)"),
        Line::from("  Tab           Toggle search scope"),
        Line::from("  Up/Down       Navigate results"),
        Line::from("  x             Clear search results"),
        Line::from(""),
        Line::from("SORT MODE").bold(),
        Line::from(""),
        Line::from("  j/k           Move selection"),
        Line::from("  Enter         Apply sort"),
        Line::from("  1-5           Quick select"),
        Line::from("  Esc           Cancel"),
    ];

    let max_scroll = help_text.len().saturating_sub(inner.height as usize);
    let actual_scroll = scroll.min(max_scroll);

    let scroll = u16::try_from(actual_scroll).unwrap_or(u16::MAX);
    let paragraph = Paragraph::new(help_text).scroll((scroll, 0));
    f.render_widget(paragraph, inner);
}

fn draw_sort_overlay(f: &mut Frame, app: &App) {
    let area = centered_rect(40, 30, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Sort By ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = SortOrder::all()
        .iter()
        .enumerate()
        .map(|(i, order)| {
            let is_selected = i == app.sort_selection;
            let is_current = *order == app.sort_order;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let marker = if is_current { " * " } else { "   " };
            ListItem::new(format!("{}{}. {}", marker, i + 1, order.label())).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn draw_search_overlay(f: &mut Frame, query: &str, cursor: usize, scope: SearchScope) {
    let area = Rect {
        x: f.area().x,
        y: f.area().height.saturating_sub(3),
        width: f.area().width,
        height: 3,
    };

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(
            " Search (scope: {}, Enter to search, Esc to exit input) ",
            scope.label()
        ))
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Build the query line with cursor
    let before_cursor = &query[..cursor];
    let cursor_char = query.chars().nth(cursor).unwrap_or(' ');
    let after_cursor = if cursor < query.len() {
        &query[cursor + 1..]
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::raw(before_cursor),
        Span::styled(
            cursor_char.to_string(),
            Style::default().bg(Color::White).fg(Color::Black),
        ),
        Span::raw(after_cursor),
    ]);

    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, inner);
}

fn draw_delete_overlay(f: &mut Frame, count: usize) {
    let area = centered_rect(50, 20, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::from(""),
        Line::from(format!("Delete {count} conversation(s)?")).bold(),
        Line::from(""),
        Line::from("This action cannot be undone.").fg(Color::DarkGray),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::default().fg(Color::Black).bg(Color::Red)),
            Span::raw(" Yes  "),
            Span::styled(" n ", Style::default().fg(Color::Black).bg(Color::Green)),
            Span::raw(" No"),
        ]),
    ];

    let paragraph = Paragraph::new(text).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(paragraph, inner);
}

fn draw_delete_source_overlay(f: &mut Frame, source_name: &str) {
    let area = centered_rect(60, 25, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm Delete Source ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::from(""),
        Line::from(format!("Delete source '{source_name}'?")).bold(),
        Line::from(""),
        Line::from("All conversations from this source will be deleted.").fg(Color::Yellow),
        Line::from("This action cannot be undone.").fg(Color::DarkGray),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::default().fg(Color::Black).bg(Color::Red)),
            Span::raw(" Yes  "),
            Span::styled(" n ", Style::default().fg(Color::Black).bg(Color::Green)),
            Span::raw(" No"),
        ]),
    ];

    let paragraph = Paragraph::new(text).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
