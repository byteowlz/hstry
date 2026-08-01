//! Application state: data, grouping, sorting, filtering, selection.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Datelike, Local, Utc};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;
use uuid::Uuid;

use hstry_core::{
    Config, Database,
    db::ListConversationsOptions,
    models::{Conversation, Message, SearchHit, Source},
};

use crate::actions::PaletteState;
use crate::images::{ImageEntry, ViewerState};
use crate::theme::adapter_color;

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
            Self::Date => "date",
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
    Sidebar,
    Chat,
}

/// Sustained modes only: Normal plus the two text-entry surfaces.
#[derive(Debug, Clone)]
pub enum Mode {
    Normal,
    /// Full-text search across the database (and remotes).
    Search {
        query: String,
        cursor: usize,
    },
    /// Quick substring filter of the visible session list.
    Filter {
        query: String,
        cursor: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    Local,
    Remote,
    All,
}

impl SearchScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::All => "all",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Local => Self::All,
            Self::All => Self::Remote,
            Self::Remote => Self::Local,
        }
    }
}

/// Transient overlays. They stack conceptually; we keep at most one.
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
    ImageViewer(Box<ViewerState>),
}

// =============================================================================
// Sidebar rows
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
    /// Index into `visible` (which indexes `all_conversations`).
    Session { vis: usize, depth: u8 },
    /// Index into `search_results`.
    Hit { idx: usize },
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
    pub sidebar_visible: bool,
    pub pending_prefix: Option<char>,

    pub group_by: GroupBy,
    pub sort_order: SortOrder,
    pub list_filter: String,
    pub collapsed: HashSet<String>,

    // Data
    pub sources: Vec<Source>,
    pub all_conversations: Vec<Conversation>,
    /// Indices into `all_conversations` after filter + sort.
    pub visible: Vec<usize>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub marked: HashSet<Uuid>,

    // Search
    pub search_results: Vec<SearchHit>,
    pub show_search_results: bool,
    pub last_search_query: Option<String>,
    pub search_scope: SearchScope,

    // Chat
    pub loaded_conversation: Option<Uuid>,
    pub messages: Vec<Message>,
    pub chat_lines: Vec<Line<'static>>,
    pub chat_images: Vec<ImageEntry>,
    pub chat_scroll: usize,
    pub chat_height: u16,

    // Layout feedback (for mouse routing)
    pub sidebar_area: Rect,
    pub chat_area: Rect,

    // Feedback
    pub status: Option<(String, Instant)>,

    // Exit request: launch `hstry resume <id> [--agent x]` after teardown.
    pub pending_resume: Option<(Uuid, Option<String>)>,

    pub image_picker: ratatui_image::picker::Picker,
}

impl App {
    pub fn new(
        config: Config,
        db: Database,
        sources: Vec<Source>,
        conversations: Vec<Conversation>,
        image_picker: ratatui_image::picker::Picker,
    ) -> Self {
        let mut app = Self {
            config,
            db,
            mode: Mode::Normal,
            overlay: None,
            focus: Focus::Sidebar,
            sidebar_visible: true,
            pending_prefix: None,
            group_by: GroupBy::Agent,
            sort_order: SortOrder::DateDesc,
            list_filter: String::new(),
            collapsed: HashSet::new(),
            sources,
            all_conversations: conversations,
            visible: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            search_results: Vec::new(),
            show_search_results: false,
            last_search_query: None,
            search_scope: SearchScope::Local,
            loaded_conversation: None,
            messages: Vec::new(),
            chat_lines: Vec::new(),
            chat_images: Vec::new(),
            chat_scroll: 0,
            chat_height: 0,
            sidebar_area: Rect::default(),
            chat_area: Rect::default(),
            status: None,
            pending_resume: None,
            image_picker,
        };
        app.rebuild();
        app
    }

    pub fn adapter_of(&self, source_id: &str) -> &str {
        self.sources
            .iter()
            .find(|s| s.id == source_id)
            .map_or("unknown", |s| s.adapter.as_str())
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    // -------------------------------------------------------------------------
    // List building
    // -------------------------------------------------------------------------

    /// Recompute `visible` and `rows`, keeping the cursor on the same session
    /// when possible.
    pub fn rebuild(&mut self) {
        let keep = self.cursor_conversation_id();

        let filter = self.list_filter.to_lowercase();
        let adapter_by_source: HashMap<&str, &str> = self
            .sources
            .iter()
            .map(|s| (s.id.as_str(), s.adapter.as_str()))
            .collect();

        self.visible = self
            .all_conversations
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                if filter.is_empty() {
                    return true;
                }
                let title = c.title.as_deref().unwrap_or("").to_lowercase();
                let ws = c.workspace.as_deref().unwrap_or("").to_lowercase();
                let adapter = adapter_by_source
                    .get(c.source_id.as_str())
                    .copied()
                    .unwrap_or("")
                    .to_lowercase();
                title.contains(&filter) || ws.contains(&filter) || adapter.contains(&filter)
            })
            .map(|(i, _)| i)
            .collect();

        let convs = &self.all_conversations;
        match self.sort_order {
            SortOrder::DateDesc => self
                .visible
                .sort_by_key(|&i| std::cmp::Reverse(convs[i].created_at)),
            SortOrder::DateAsc => self.visible.sort_by_key(|&i| convs[i].created_at),
            SortOrder::TitleAsc => self.visible.sort_by(|&a, &b| {
                convs[a]
                    .title
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&convs[b].title.as_deref().unwrap_or("").to_lowercase())
            }),
        }

        self.rows = if self.show_search_results {
            (0..self.search_results.len())
                .map(|idx| Row::Hit { idx })
                .collect()
        } else {
            self.build_grouped_rows()
        };

        // Restore cursor
        self.cursor = keep
            .and_then(|id| {
                self.rows.iter().position(|r| match r {
                    Row::Session { vis, .. } => self
                        .visible
                        .get(*vis)
                        .is_some_and(|&ci| self.all_conversations[ci].id == id),
                    Row::Hit { idx } => self
                        .search_results
                        .get(*idx)
                        .is_some_and(|h| h.conversation_id == id),
                    Row::Header { .. } => false,
                })
            })
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
    }

    fn build_grouped_rows(&self) -> Vec<Row> {
        match self.group_by {
            GroupBy::Flat => self
                .visible
                .iter()
                .enumerate()
                .map(|(vis, _)| Row::Session { vis, depth: 0 })
                .collect(),
            GroupBy::Agent => self.group_by_key(|app, ci| {
                let adapter = app.adapter_of(&app.all_conversations[ci].source_id);
                (adapter.to_string(), adapter.to_string())
            }),
            GroupBy::Repo => self.group_by_key(|app, ci| {
                let ws = app.all_conversations[ci].workspace.clone();
                let label = ws.as_deref().map_or("(no repo)".into(), shorten_workspace);
                (ws.unwrap_or_else(|| "\u{0}none".into()), label)
            }),
            GroupBy::Date => self.group_by_key(|app, ci| {
                let dt = app.all_conversations[ci].created_at;
                let local: DateTime<Local> = dt.into();
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
        for (vis, &ci) in self.visible.iter().enumerate() {
            let (key, label) = key_of(self, ci);
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

        for (vis, &ci) in self.visible.iter().enumerate() {
            let conv = &self.all_conversations[ci];
            let adapter = self.adapter_of(&conv.source_id).to_string();
            let repo = conv
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

    pub fn cursor_conversation_id(&self) -> Option<Uuid> {
        match self.rows.get(self.cursor)? {
            Row::Session { vis, .. } => self
                .visible
                .get(*vis)
                .and_then(|&ci| self.all_conversations.get(ci))
                .map(|c| c.id),
            Row::Hit { idx } => self.search_results.get(*idx).map(|h| h.conversation_id),
            Row::Header { .. } => None,
        }
    }

    pub fn cursor_conversation(&self) -> Option<&Conversation> {
        let id = self.cursor_conversation_id()?;
        self.all_conversations.iter().find(|c| c.id == id)
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
                    ids.push(self.all_conversations[self.visible[*vis]].id);
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
    // Data loading
    // -------------------------------------------------------------------------

    pub fn refresh_data(&mut self, rt: &tokio::runtime::Runtime) {
        match rt.block_on(self.db.list_sources()) {
            Ok(sources) => self.sources = sources,
            Err(e) => self.set_status(format!("Error loading sources: {e}")),
        }
        match rt.block_on(self.db.list_conversations(ListConversationsOptions {
            limit: None,
            ..Default::default()
        })) {
            Ok(convs) => {
                self.all_conversations = convs;
                self.show_search_results = false;
                self.search_results.clear();
                self.last_search_query = None;
                self.rebuild();
                self.set_status("Refreshed");
            }
            Err(e) => self.set_status(format!("Error loading conversations: {e}")),
        }
    }

    pub fn load_messages(&mut self, rt: &tokio::runtime::Runtime) {
        // Remote hit: fetch from the remote host.
        if self.show_search_results
            && let Some(Row::Hit { idx }) = self.rows.get(self.cursor)
            && let Some(hit) = self.search_results.get(*idx)
            && let Some(host) = hit.host.clone()
        {
            let conv_id = hit.conversation_id;
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
                    Err(e) => {
                        self.set_status(format!("Remote load error: {e}"));
                        self.messages.clear();
                        self.after_messages_loaded();
                        return;
                    }
                }
            }
            self.set_status(format!("Remote '{host}' not found in config"));
            self.messages.clear();
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

    fn after_messages_loaded(&mut self) {
        let highlight = if self.show_search_results {
            self.last_search_query.clone()
        } else {
            None
        };
        let (lines, images) =
            crate::ui::chat::build_chat_lines(&self.messages, highlight.as_deref());
        self.chat_lines = lines;
        self.chat_images = images;
        self.chat_scroll = highlight
            .as_deref()
            .and_then(|q| first_match_line(&self.chat_lines, q))
            .unwrap_or(0);
    }

    pub fn perform_search(&mut self, query: &str, rt: &tokio::runtime::Runtime) {
        if query.is_empty() {
            self.search_results.clear();
            self.show_search_results = false;
            self.last_search_query = None;
            self.rebuild();
            return;
        }

        let opts = hstry_core::db::SearchOptions {
            limit: Some(200),
            ..Default::default()
        };
        let scope = self.search_scope;
        let config = &self.config;
        let db = &self.db;
        let q = query.to_string();

        let search = async {
            let mut results = Vec::new();
            if scope != SearchScope::Remote {
                let local =
                    if let Some(hits) = hstry_core::service::try_service_search(&q, &opts).await? {
                        hits
                    } else {
                        db.search(&q, opts.clone()).await?
                    };
                results.extend(local);
            }
            if scope != SearchScope::Local {
                let remote_hits =
                    hstry_core::remote::search_remotes(&config.remotes, &q, &opts).await?;
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
                let n = results.len();
                self.search_results = results;
                self.show_search_results = true;
                self.last_search_query = Some(query.to_string());
                self.rebuild();
                self.cursor = 0;
                self.loaded_conversation = None;
                self.set_status(format!("{n} results ({})", scope.label()));
            }
            Err(e) => self.set_status(format!("Search error: {e}")),
        }
    }

    pub fn clear_search(&mut self) {
        self.show_search_results = false;
        self.search_results.clear();
        self.last_search_query = None;
        self.loaded_conversation = None;
        self.rebuild();
    }
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

/// Compact relative timestamp for list rows.
pub fn relative_time(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(dt);
    if delta.num_minutes() < 1 {
        "now".to_string()
    } else if delta.num_hours() < 1 {
        format!("{}m", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h", delta.num_hours())
    } else if delta.num_days() < 7 {
        format!("{}d", delta.num_days())
    } else {
        let local: DateTime<Local> = dt.into();
        if local.year() == Local::now().year() {
            local.format("%b %d").to_string()
        } else {
            local.format("%Y-%m-%d").to_string()
        }
    }
}

fn first_match_line(lines: &[Line<'static>], query: &str) -> Option<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    lines.iter().position(|line| {
        line.spans
            .iter()
            .any(|span| span.content.as_ref().to_lowercase().contains(&needle))
    })
}
