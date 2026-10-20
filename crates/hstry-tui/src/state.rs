//! Application state: data, the live query, grouping, sorting, selection.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, Utc};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;
use uuid::Uuid;

use hstry_core::{
    Config, Database,
    db::{ConversationSummary, ListConversationsOptions},
    models::{Conversation, Message, SearchHit, Source},
};

use crate::actions::PaletteState;
use crate::fuzzy;
use crate::images::{ImageEntry, ViewerState};
use crate::theme::adapter_color;

/// How long the query must be idle before a content search runs.
pub const CONTENT_SEARCH_DEBOUNCE: Duration = Duration::from_millis(220);

/// Terminals narrower than this show one pane at a time.
pub const NARROW_WIDTH: u16 = 100;

// =============================================================================
// Enums
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Flat,
    Agent,
    Repo,
    AgentRepo,
    Date,
}

impl GroupBy {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Agent => "agent",
            Self::Repo => "repo",
            Self::AgentRepo => "agent▸repo",
            Self::Date => "month",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    DateDesc,
    DateAsc,
    TitleAsc,
}

impl SortOrder {
    pub const fn label(self) -> &'static str {
        match self {
            Self::DateDesc => "newest",
            Self::DateAsc => "oldest",
            Self::TitleAsc => "title",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Transcript,
}

/// Sustained modes: Normal, or typing into the query bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Query,
}

/// What the query bar searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Instant fuzzy match over title, repo, agent and first prompt.
    Sessions,
    /// Full-text search over every message (debounced).
    Content,
}

impl Scope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Content => "content",
        }
    }

    pub const fn toggle(self) -> Self {
        match self {
            Self::Sessions => Self::Content,
            Self::Content => Self::Sessions,
        }
    }
}

/// Export formats offered by the export overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
}

impl ExportFormat {
    pub const ALL: [Self; 2] = [Self::Markdown, Self::Json];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown transcript (.md)",
            Self::Json => "JSON (conversation + messages)",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
        }
    }
}

/// Transient overlays. At most one is open.
pub enum Overlay {
    Help {
        scroll: usize,
    },
    Palette(PaletteState),
    ConfirmDelete {
        ids: Vec<Uuid>,
    },
    ConfirmDeleteSource {
        source_id: String,
        name: String,
    },
    AgentPicker {
        conv: Uuid,
        agents: Vec<String>,
        cursor: usize,
    },
    ExportPicker {
        conv: Uuid,
        cursor: usize,
    },
    ImageViewer(Box<ViewerState>),
}

// =============================================================================
// Data
// =============================================================================

/// A session plus the derived fields the list needs.
pub struct Session {
    pub conv: Conversation,
    pub adapter: String,
    pub first_message: Option<String>,
    pub message_count: i64,
    /// Lowercase-agnostic secondary match text: repo, agent, first prompt.
    pub secondary: String,
}

impl Session {
    pub fn title(&self) -> &str {
        self.conv
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .or(self.first_message.as_deref().map(str::trim))
            .unwrap_or("(untitled)")
    }

    pub fn last_active(&self) -> DateTime<Utc> {
        self.conv.updated_at.unwrap_or(self.conv.created_at)
    }

    /// Best on-disk location for this session: the per-session file when the
    /// adapter recorded one, otherwise nothing (the source path is a root).
    pub fn file_path(&self) -> Option<String> {
        self.conv
            .metadata
            .get("file")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    }
}

/// One visible session with its title highlight positions.
#[derive(Debug, Clone)]
pub struct Visible {
    pub idx: usize,
    pub title_positions: Vec<usize>,
}

/// Content-search hits grouped by session (best hit first).
pub struct SessionHits {
    pub conversation_id: Uuid,
    pub hits: Vec<SearchHit>,
}

impl SessionHits {
    pub fn best(&self) -> &SearchHit {
        &self.hits[0]
    }
}

// =============================================================================
// List rows
// =============================================================================

#[derive(Debug, Clone)]
pub enum Row {
    Header {
        key: String,
        label: String,
        count: usize,
        depth: u8,
        collapsed: bool,
        color: Option<Color>,
        /// Set when this header maps 1:1 to a deletable source.
        source_id: Option<String>,
    },
    /// Index into `visible`.
    Session { vis: usize, depth: u8 },
    /// Index into `content_results`.
    Hits { idx: usize },
}

// =============================================================================
// App
// =============================================================================

pub struct App {
    pub config: Config,
    pub db: Database,

    pub mode: Mode,
    pub overlay: Option<Overlay>,
    pub focus: Focus,
    pub pending_prefix: Option<char>,
    /// Set by the renderer: true when only one pane fits.
    pub narrow: bool,

    pub group_by: GroupBy,
    pub sort_order: SortOrder,
    pub collapsed: HashSet<String>,

    // Query
    pub query: String,
    pub query_cursor: usize,
    pub scope: Scope,
    /// When the query last changed; drives the content-search debounce.
    pub query_changed_at: Option<Instant>,
    /// The query the current content results belong to.
    pub content_query: Option<String>,
    pub content_results: Vec<SessionHits>,
    pub searching: bool,

    // Data
    pub sources: Vec<Source>,
    pub sessions: Vec<Session>,
    pub visible: Vec<Visible>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub marked: HashSet<Uuid>,

    // Transcript
    pub loaded_conversation: Option<Uuid>,
    pub messages: Vec<Message>,
    pub chat_lines: Vec<Line<'static>>,
    pub chat_images: Vec<ImageEntry>,
    pub chat_scroll: usize,
    pub chat_height: u16,
    /// Content width the transcript lines were wrapped for; 0 = unwrapped.
    pub chat_wrap_width: usize,
    /// Show thinking and tool output in full instead of a short preview.
    pub expand_details: bool,

    // Layout feedback (for mouse routing)
    pub list_area: Rect,
    pub chat_area: Rect,

    // Feedback
    pub status: Option<(String, Instant)>,

    /// Exit request: launch `hstry resume <id> [--agent x]` after teardown.
    pub pending_resume: Option<(Uuid, Option<String>)>,

    pub image_picker: ratatui_image::picker::Picker,
}

impl App {
    pub fn new(
        config: Config,
        db: Database,
        sources: Vec<Source>,
        summaries: Vec<ConversationSummary>,
        image_picker: ratatui_image::picker::Picker,
    ) -> Self {
        let mut app = Self {
            config,
            db,
            mode: Mode::Normal,
            overlay: None,
            focus: Focus::List,
            pending_prefix: None,
            narrow: false,
            group_by: GroupBy::Flat,
            sort_order: SortOrder::DateDesc,
            collapsed: HashSet::new(),
            query: String::new(),
            query_cursor: 0,
            scope: Scope::Sessions,
            query_changed_at: None,
            content_query: None,
            content_results: Vec::new(),
            searching: false,
            sessions: Vec::new(),
            sources,
            visible: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            loaded_conversation: None,
            messages: Vec::new(),
            chat_lines: Vec::new(),
            chat_images: Vec::new(),
            chat_scroll: 0,
            chat_height: 0,
            chat_wrap_width: 0,
            expand_details: false,
            list_area: Rect::default(),
            chat_area: Rect::default(),
            status: None,
            pending_resume: None,
            image_picker,
        };
        app.set_sessions(summaries);
        app.rebuild();
        app
    }

    fn set_sessions(&mut self, summaries: Vec<ConversationSummary>) {
        let adapters: HashMap<String, String> = self
            .sources
            .iter()
            .map(|s| (s.id.clone(), s.adapter.clone()))
            .collect();
        self.sessions = summaries
            .into_iter()
            .map(|summary| {
                let adapter = adapters
                    .get(&summary.conversation.source_id)
                    .cloned()
                    .unwrap_or_else(|| summary.conversation.source_id.clone());
                let first: Option<String> = summary
                    .first_user_message
                    .map(|m| m.chars().take(400).collect::<String>())
                    .map(|m| m.split_whitespace().collect::<Vec<_>>().join(" "));
                let secondary = format!(
                    "{} {} {}",
                    summary.conversation.workspace.as_deref().unwrap_or(""),
                    adapter,
                    first.as_deref().unwrap_or("")
                );
                Session {
                    adapter,
                    first_message: first,
                    message_count: summary
                        .message_count
                        .max(summary.conversation.message_count),
                    secondary,
                    conv: summary.conversation,
                }
            })
            .collect();
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn session_by_id(&self, id: Uuid) -> Option<&Session> {
        self.sessions.iter().find(|s| s.conv.id == id)
    }

    /// True when the list is showing grouped content-search results.
    pub fn showing_content(&self) -> bool {
        self.scope == Scope::Content
            && self.content_query.is_some()
            && !self.query.trim().is_empty()
    }

    // -------------------------------------------------------------------------
    // List building
    // -------------------------------------------------------------------------

    /// Recompute `visible` and `rows`, keeping the cursor on the same session
    /// when possible.
    pub fn rebuild(&mut self) {
        let keep = self.cursor_conversation_id();
        let query = if self.scope == Scope::Sessions {
            self.query.trim().to_string()
        } else {
            String::new()
        };

        let mut scored: Vec<(i64, Visible)> = self
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(idx, s)| {
                let m = fuzzy::match_query(
                    &query,
                    &fuzzy::Target {
                        title: s.title(),
                        secondary: &s.secondary,
                    },
                )?;
                Some((
                    m.score,
                    Visible {
                        idx,
                        title_positions: m.title_positions,
                    },
                ))
            })
            .collect();

        let sessions = &self.sessions;
        if query.is_empty() {
            match self.sort_order {
                SortOrder::DateDesc => {
                    scored.sort_by_key(|(_, v)| std::cmp::Reverse(sessions[v.idx].last_active()));
                }
                SortOrder::DateAsc => scored.sort_by_key(|(_, v)| sessions[v.idx].last_active()),
                SortOrder::TitleAsc => scored.sort_by(|(_, a), (_, b)| {
                    sessions[a.idx]
                        .title()
                        .to_lowercase()
                        .cmp(&sessions[b.idx].title().to_lowercase())
                }),
            }
        } else {
            // Relevance first, recency breaks ties.
            scored.sort_by(|(sa, a), (sb, b)| {
                sb.cmp(sa).then_with(|| {
                    sessions[b.idx]
                        .last_active()
                        .cmp(&sessions[a.idx].last_active())
                })
            });
        }
        self.visible = scored.into_iter().map(|(_, v)| v).collect();

        self.rows = if self.showing_content() {
            (0..self.content_results.len())
                .map(|idx| Row::Hits { idx })
                .collect()
        } else if !query.is_empty() {
            // A live query is a ranked list; groups would hide the best match.
            (0..self.visible.len())
                .map(|vis| Row::Session { vis, depth: 0 })
                .collect()
        } else {
            self.build_grouped_rows()
        };

        self.cursor = keep
            .and_then(|id| self.row_of(id))
            .or_else(|| self.first_selectable_row())
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
    }

    fn row_of(&self, id: Uuid) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| self.row_conversation_id(r) == Some(id))
    }

    pub fn first_selectable_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| !matches!(r, Row::Header { .. }))
    }

    fn build_grouped_rows(&self) -> Vec<Row> {
        match self.group_by {
            GroupBy::Flat => (0..self.visible.len())
                .map(|vis| Row::Session { vis, depth: 0 })
                .collect(),
            GroupBy::Agent => self.group_by_key(|app, s| {
                let a = &app.sessions[s].adapter;
                (a.clone(), a.clone())
            }),
            GroupBy::Repo => self.group_by_key(|app, s| {
                let ws = app.sessions[s].conv.workspace.clone();
                let label = ws.as_deref().map_or("(no repo)".into(), shorten_workspace);
                (ws.unwrap_or_else(|| "\u{0}none".into()), label)
            }),
            GroupBy::Date => self.group_by_key(|app, s| {
                let local: DateTime<Local> = app.sessions[s].last_active().into();
                (
                    format!("{}-{:02}", local.year(), local.month()),
                    local.format("%B %Y").to_string(),
                )
            }),
            GroupBy::AgentRepo => self.group_agent_repo(),
        }
    }

    /// One-level grouping preserving `visible` order inside groups.
    fn group_by_key(&self, key_of: impl Fn(&Self, usize) -> (String, String)) -> Vec<Row> {
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, (String, Vec<usize>)> = HashMap::new();
        for (vis, v) in self.visible.iter().enumerate() {
            let (key, label) = key_of(self, v.idx);
            if !groups.contains_key(&key) {
                order.push(key.clone());
            }
            groups
                .entry(key)
                .or_insert_with(|| (label, Vec::new()))
                .1
                .push(vis);
        }

        let mut rows = Vec::new();
        for key in order {
            let Some((label, members)) = groups.remove(&key) else {
                continue;
            };
            let collapsed = self.collapsed.contains(&key);
            let color = (self.group_by == GroupBy::Agent).then(|| adapter_color(&label));
            let source_id = (self.group_by == GroupBy::Agent)
                .then(|| self.single_source_for_adapter(&label))
                .flatten();
            rows.push(Row::Header {
                key,
                label,
                count: members.len(),
                depth: 0,
                collapsed,
                color,
                source_id,
            });
            if !collapsed {
                rows.extend(
                    members
                        .into_iter()
                        .map(|vis| Row::Session { vis, depth: 1 }),
                );
            }
        }
        rows
    }

    fn group_agent_repo(&self) -> Vec<Row> {
        let mut agent_order: Vec<String> = Vec::new();
        let mut agents: HashMap<String, (Vec<String>, HashMap<String, Vec<usize>>)> =
            HashMap::new();

        for (vis, v) in self.visible.iter().enumerate() {
            let session = &self.sessions[v.idx];
            let adapter = session.adapter.clone();
            let repo = session
                .conv
                .workspace
                .as_deref()
                .map_or("(no repo)".to_string(), shorten_workspace);
            if !agents.contains_key(&adapter) {
                agent_order.push(adapter.clone());
            }
            let entry = agents.entry(adapter).or_default();
            if !entry.1.contains_key(&repo) {
                entry.0.push(repo.clone());
            }
            entry.1.entry(repo).or_default().push(vis);
        }

        let mut rows = Vec::new();
        for adapter in agent_order {
            let Some((repo_order, mut repos)) = agents.remove(&adapter) else {
                continue;
            };
            let count: usize = repos.values().map(Vec::len).sum();
            let a_key = format!("a:{adapter}");
            let a_collapsed = self.collapsed.contains(&a_key);
            rows.push(Row::Header {
                key: a_key,
                label: adapter.clone(),
                count,
                depth: 0,
                collapsed: a_collapsed,
                color: Some(adapter_color(&adapter)),
                source_id: self.single_source_for_adapter(&adapter),
            });
            if a_collapsed {
                continue;
            }
            for repo in repo_order {
                let Some(members) = repos.remove(&repo) else {
                    continue;
                };
                let r_key = format!("r:{adapter}/{repo}");
                let r_collapsed = self.collapsed.contains(&r_key);
                rows.push(Row::Header {
                    key: r_key,
                    label: repo,
                    count: members.len(),
                    depth: 1,
                    collapsed: r_collapsed,
                    color: None,
                    source_id: None,
                });
                if !r_collapsed {
                    rows.extend(
                        members
                            .into_iter()
                            .map(|vis| Row::Session { vis, depth: 2 }),
                    );
                }
            }
        }
        rows
    }

    fn single_source_for_adapter(&self, adapter: &str) -> Option<String> {
        let mut ids = self.sources.iter().filter(|s| s.adapter == adapter);
        let first = ids.next()?;
        if ids.next().is_none() {
            Some(first.id.clone())
        } else {
            None
        }
    }

    // -------------------------------------------------------------------------
    // Cursor helpers
    // -------------------------------------------------------------------------

    pub fn cursor_row(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    fn row_conversation_id(&self, row: &Row) -> Option<Uuid> {
        match row {
            Row::Session { vis, .. } => self
                .visible
                .get(*vis)
                .and_then(|v| self.sessions.get(v.idx))
                .map(|s| s.conv.id),
            Row::Hits { idx } => self.content_results.get(*idx).map(|h| h.conversation_id),
            Row::Header { .. } => None,
        }
    }

    pub fn cursor_conversation_id(&self) -> Option<Uuid> {
        self.row_conversation_id(self.rows.get(self.cursor)?)
    }

    pub fn cursor_session(&self) -> Option<&Session> {
        self.session_by_id(self.cursor_conversation_id()?)
    }

    /// The content hits for the cursor row, when showing content results.
    pub fn cursor_hits(&self) -> Option<&SessionHits> {
        match self.rows.get(self.cursor)? {
            Row::Hits { idx } => self.content_results.get(*idx),
            _ => None,
        }
    }

    pub fn header_session_ids(&self, header_idx: usize) -> Vec<Uuid> {
        let Some(Row::Header { depth, .. }) = self.rows.get(header_idx) else {
            return Vec::new();
        };
        let hdr_depth = *depth;
        let mut ids = Vec::new();
        for row in &self.rows[header_idx + 1..] {
            match row {
                Row::Header { depth, .. } if *depth <= hdr_depth => break,
                Row::Session { vis, .. } => {
                    ids.push(self.sessions[self.visible[*vis].idx].conv.id);
                }
                _ => {}
            }
        }
        ids
    }

    pub fn toggle_collapse(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
        self.rebuild();
    }

    pub fn all_group_keys(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|r| match r {
                Row::Header { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    /// Conversations targeted by a delete: marked ones, or the cursor session.
    pub fn delete_targets(&self) -> Vec<Uuid> {
        if self.marked.is_empty() {
            self.cursor_conversation_id().into_iter().collect()
        } else {
            self.marked.iter().copied().collect()
        }
    }

    // -------------------------------------------------------------------------
    // Query editing
    // -------------------------------------------------------------------------

    /// Apply a query edit: sessions re-filter instantly; content search is
    /// debounced by `tick`.
    pub fn query_edited(&mut self) {
        self.query_changed_at = Some(Instant::now());
        if self.scope == Scope::Sessions || self.query.trim().is_empty() {
            self.content_query = None;
            self.content_results.clear();
            self.rebuild();
            // Like fzf: while typing, the selection follows the best match.
            self.cursor = self.first_selectable_row().unwrap_or(0);
            self.loaded_conversation = None;
        }
    }

    pub fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.content_query = None;
        self.content_results.clear();
        self.query_changed_at = Some(Instant::now());
        self.rebuild();
        self.loaded_conversation = None;
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
        self.query_cursor = 0;
        self.content_query = None;
        self.content_results.clear();
        self.query_changed_at = None;
        self.rebuild();
        self.loaded_conversation = None;
    }

    /// Run due background work. Returns true when something changed.
    pub fn tick(&mut self, rt: &tokio::runtime::Runtime) -> bool {
        if self.scope != Scope::Content {
            return false;
        }
        let Some(changed) = self.query_changed_at else {
            return false;
        };
        if changed.elapsed() < CONTENT_SEARCH_DEBOUNCE {
            return false;
        }
        self.query_changed_at = None;
        let q = self.query.trim().to_string();
        if q.is_empty() || self.content_query.as_deref() == Some(q.as_str()) {
            return false;
        }
        self.perform_content_search(&q, rt);
        self.load_messages(rt);
        true
    }

    // -------------------------------------------------------------------------
    // Data loading
    // -------------------------------------------------------------------------

    pub fn refresh_data(&mut self, rt: &tokio::runtime::Runtime) {
        match rt.block_on(self.db.list_sources()) {
            Ok(sources) => self.sources = sources,
            Err(e) => self.set_status(format!("Error loading sources: {e}")),
        }
        match rt.block_on(
            self.db
                .list_conversation_summaries(ListConversationsOptions {
                    limit: None,
                    ..Default::default()
                }),
        ) {
            Ok(summaries) => {
                self.set_sessions(summaries);
                self.content_query = None;
                self.content_results.clear();
                self.loaded_conversation = None;
                self.rebuild();
                self.set_status(format!("Refreshed · {} sessions", self.sessions.len()));
            }
            Err(e) => self.set_status(format!("Error loading sessions: {e}")),
        }
    }

    pub fn load_messages(&mut self, rt: &tokio::runtime::Runtime) {
        // Remote hit: fetch from the remote host.
        if let Some(hits) = self.cursor_hits()
            && let Some(host) = hits.best().host.clone()
        {
            let conv_id = hits.conversation_id;
            if self.loaded_conversation == Some(conv_id) {
                return;
            }
            if let Some(remote) = self.config.remotes.iter().find(|r| r.name == host).cloned() {
                match rt.block_on(hstry_core::remote::show_remote(
                    &remote,
                    &conv_id.to_string(),
                )) {
                    Ok(details) => {
                        self.messages = details.messages.into_iter().map(|m| m.message).collect();
                        self.loaded_conversation = Some(conv_id);
                        self.after_messages_loaded();
                        return;
                    }
                    Err(e) => self.set_status(format!("Remote load error: {e}")),
                }
            } else {
                self.set_status(format!("Remote '{host}' not found in config"));
            }
            self.messages.clear();
            self.loaded_conversation = None;
            self.after_messages_loaded();
            return;
        }

        if let Some(conv_id) = self.cursor_conversation_id() {
            if self.loaded_conversation == Some(conv_id) {
                return;
            }
            match rt.block_on(self.db.get_messages(conv_id)) {
                Ok(msgs) => {
                    self.messages = msgs;
                    self.loaded_conversation = Some(conv_id);
                    self.after_messages_loaded();
                }
                Err(e) => self.set_status(format!("Error loading messages: {e}")),
            }
        } else if !matches!(self.rows.get(self.cursor), Some(Row::Header { .. })) {
            self.messages.clear();
            self.loaded_conversation = None;
            self.after_messages_loaded();
        }
    }

    fn highlight_query(&self) -> Option<String> {
        self.showing_content()
            .then(|| self.query.trim().to_string())
    }

    /// Re-render transcript lines for the current wrap width / detail level.
    pub fn rebuild_chat_lines(&mut self) {
        let highlight = self.highlight_query();
        let (lines, images) = crate::ui::chat::build_chat_lines(
            &self.messages,
            highlight.as_deref(),
            self.chat_wrap_width,
            self.expand_details,
        );
        self.chat_lines = lines;
        self.chat_images = images;
    }

    fn after_messages_loaded(&mut self) {
        self.rebuild_chat_lines();
        let highlight = self.highlight_query();
        self.chat_scroll = highlight
            .as_deref()
            .and_then(|q| first_match_line(&self.chat_lines, q))
            .map_or(0, |line| line.saturating_sub(2));
    }

    pub fn perform_content_search(&mut self, query: &str, rt: &tokio::runtime::Runtime) {
        let opts = hstry_core::db::SearchOptions {
            limit: Some(400),
            ..Default::default()
        };
        let db = &self.db;
        let q = query.to_string();
        let remotes = self.config.remotes.clone();
        let search = async {
            let mut results =
                if let Some(hits) = hstry_core::service::try_service_search(&q, &opts).await? {
                    hits
                } else {
                    db.search(&q, opts.clone()).await?
                };
            if !remotes.is_empty()
                && let Ok(remote) = hstry_core::remote::search_remotes(&remotes, &q, &opts).await
            {
                results.extend(remote.hits);
            }
            Ok::<_, hstry_core::Error>(results)
        };

        self.searching = true;
        match rt.block_on(search) {
            Ok(results) => {
                let total = results.len();
                self.content_results = group_hits(results);
                self.content_query = Some(query.to_string());
                self.loaded_conversation = None;
                self.rebuild();
                self.cursor = 0;
                self.set_status(format!(
                    "{total} hits in {} sessions",
                    self.content_results.len()
                ));
            }
            Err(e) => self.set_status(format!("Search error: {e}")),
        }
        self.searching = false;
    }
}

/// Group hits by session, best session first, best hit first within one.
pub fn group_hits(mut hits: Vec<SearchHit>) -> Vec<SessionHits> {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut order: Vec<Uuid> = Vec::new();
    let mut by_conv: HashMap<Uuid, Vec<SearchHit>> = HashMap::new();
    for hit in hits {
        if !by_conv.contains_key(&hit.conversation_id) {
            order.push(hit.conversation_id);
        }
        by_conv.entry(hit.conversation_id).or_default().push(hit);
    }
    order
        .into_iter()
        .filter_map(|id| {
            by_conv.remove(&id).map(|hits| SessionHits {
                conversation_id: id,
                hits,
            })
        })
        .collect()
}

// =============================================================================
// Helpers
// =============================================================================

pub fn shorten_workspace(ws: &str) -> String {
    let trimmed = ws.trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => "/".to_string(),
        1 => parts[0].to_string(),
        _ => parts[parts.len() - 2..].join("/"),
    }
}

/// Last path component of a workspace: the repo name users recognize.
pub fn repo_name(ws: &str) -> String {
    ws.trim_end_matches('/')
        .rsplit('/')
        .find(|p| !p.is_empty())
        .unwrap_or(ws)
        .to_string()
}

/// Compact relative timestamp for list rows.
pub fn relative_time(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(dt);
    if delta.num_minutes() < 1 {
        "now".to_string()
    } else if delta.num_hours() < 1 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_days() < 7 {
        format!("{}d ago", delta.num_days())
    } else {
        let local: DateTime<Local> = dt.into();
        if local.year() == Local::now().year() {
            local.format("%b %-d").to_string()
        } else {
            local.format("%Y-%m-%d").to_string()
        }
    }
}

/// A short, word-aligned excerpt of `text` around the first query term.
pub fn excerpt(text: &str, query: &str, width: usize) -> String {
    // Markdown fences and heading marks are noise in a one-line excerpt.
    let flat: String = text
        .split_whitespace()
        .filter(|w| !w.starts_with("```") && !w.chars().all(|c| c == '#'))
        .collect::<Vec<_>>()
        .join(" ");
    let lower = flat.to_lowercase();
    let hit = query
        .split_whitespace()
        .filter_map(|t| lower.find(&t.to_lowercase()))
        .min();
    let Some(hit) = hit else {
        return crate::markdown::truncate_str(&flat, width);
    };
    // Start a few words before the hit, on a word boundary.
    let lead = width / 4;
    let mut start = hit.saturating_sub(lead);
    while start > 0 && !flat.is_char_boundary(start) {
        start -= 1;
    }
    if start > 0 {
        start = flat[..start].rfind(' ').map_or(0, |i| i + 1);
    }
    let body = &flat[start..];
    let prefix = if start > 0 { "…" } else { "" };
    crate::markdown::truncate_str(&format!("{prefix}{body}"), width)
}

fn first_match_line(lines: &[Line<'static>], query: &str) -> Option<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return None;
    }
    lines.iter().position(|line| {
        let text: String = line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .to_lowercase();
        terms.iter().any(|t| text.contains(t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_starts_on_a_word_boundary() {
        let text = "The snippet() call re-tokenized huge tool blobs. Selecting the evidence window first cut p95";
        let out = excerpt(text, "evidence", 40);
        assert!(out.starts_with('…'), "{out}");
        assert!(out.contains("evidence"), "{out}");
        assert!(
            !out.contains("electing the") || out.contains("Selecting"),
            "cut mid-word: {out}"
        );
    }

    #[test]
    fn excerpt_drops_markdown_fences() {
        let out = excerpt("Done.\n\n```rust\nfn backoff() {}\n```", "backoff", 60);
        assert!(!out.contains("```"), "{out}");
        assert!(out.contains("fn backoff"), "{out}");
    }

    #[test]
    fn excerpt_without_hit_truncates() {
        assert_eq!(excerpt("short text", "zzz", 40), "short text");
    }

    #[test]
    fn repo_name_is_last_component() {
        assert_eq!(repo_name("/work/hstry/"), "hstry");
        assert_eq!(repo_name("hstry"), "hstry");
    }
}
