#![allow(clippy::print_stdout, clippy::print_stderr)]

//! hstry TUI: read past agent sessions fast, jump back in with a coding agent.

mod actions;
mod images;
mod markdown;
mod state;
mod theme;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use uuid::Uuid;

use hstry_core::{Config, Database, db::ListConversationsOptions};

use actions::{ActionId, PaletteState, resolve_progression};
use images::ViewerState;
use state::{App, Focus, GroupBy, Mode, Overlay, Row, SortOrder};

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

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

/// RAII terminal teardown: restore on drop, even on panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .common
        .config
        .unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    let rt = tokio::runtime::Runtime::new()?;
    let db = rt.block_on(Database::open(&config.database))?;
    let sources = rt.block_on(db.list_sources())?;
    let conversations = rt.block_on(db.list_conversations(ListConversationsOptions {
        limit: None,
        ..Default::default()
    }))?;

    let picker = images::detect_picker();

    let mut app = App::new(config, db, sources, conversations, picker);

    let result = {
        let _guard = TerminalGuard::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        run_app(&mut terminal, &mut app, &rt)
    };

    let pending = app.pending_resume.take();
    rt.block_on(app.db.close());
    result?;

    if let Some((id, agent)) = pending {
        let mut cmd = std::process::Command::new("hstry");
        cmd.arg("resume").arg(id.to_string());
        if let Some(agent) = agent {
            cmd.args(["--agent", &agent]);
        }
        let status = cmd.status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("hstry resume exited with {s}"),
            Err(e) => eprintln!("Failed to launch `hstry resume`: {e}"),
        }
    }

    Ok(())
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rt: &tokio::runtime::Runtime,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    if !app.rows.is_empty() {
        // Land on the first session, not a group header.
        if let Some(idx) = app
            .rows
            .iter()
            .position(|r| matches!(r, Row::Session { .. }))
        {
            app.cursor = idx;
        }
        app.load_messages(rt);
    }

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        match event::read()? {
            Event::Key(key)
                if key.kind == KeyEventKind::Press && handle_key(app, &key, rt) == Flow::Quit =>
            {
                return Ok(());
            }
            Event::Mouse(mouse) => handle_mouse(app, mouse.kind, mouse.column, mouse.row, rt),
            _ => {}
        }
    }
}

fn handle_mouse(
    app: &mut App,
    kind: MouseEventKind,
    col: u16,
    row: u16,
    rt: &tokio::runtime::Runtime,
) {
    let in_chat = app
        .chat_area
        .contains(ratatui::layout::Position::new(col, row));
    match kind {
        MouseEventKind::ScrollDown => {
            if in_chat {
                app.chat_scroll = app.chat_scroll.saturating_add(3);
            } else {
                move_cursor(app, 1, rt);
            }
        }
        MouseEventKind::ScrollUp => {
            if in_chat {
                app.chat_scroll = app.chat_scroll.saturating_sub(3);
            } else {
                move_cursor(app, -1, rt);
            }
        }
        _ => {}
    }
}

fn handle_key(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) -> Flow {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Flow::Quit;
    }

    if app.overlay.is_some() {
        return handle_overlay(app, key, rt);
    }

    match app.mode {
        Mode::Normal => handle_normal(app, key, rt),
        Mode::Search { .. } => {
            handle_search_input(app, key, rt);
            Flow::Continue
        }
        Mode::Filter { .. } => {
            handle_filter_input(app, key);
            Flow::Continue
        }
    }
}

// =============================================================================
// Normal mode
// =============================================================================

fn handle_normal(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) -> Flow {
    // Complete or cancel a pending key progression.
    if let Some(prefix) = app.pending_prefix.take() {
        if let KeyCode::Char(c) = key.code {
            if prefix == 'g' && c == 'g' {
                jump_top(app, rt);
                return Flow::Continue;
            }
            if let Some(action) = resolve_progression(prefix, c) {
                return run_action(app, action, rt);
            }
        }
        return Flow::Continue;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('p') if ctrl => {
            app.overlay = Some(Overlay::Palette(PaletteState::new()));
        }
        KeyCode::Char('r') if ctrl => return run_action(app, ActionId::Resume, rt),
        KeyCode::Char('a') if ctrl => return run_action(app, ActionId::MarkAll, rt),
        KeyCode::Char('d') if ctrl => half_page(app, 1, rt),
        KeyCode::Char('u') if ctrl => half_page(app, -1, rt),
        KeyCode::Char('q') => return Flow::Quit,
        KeyCode::Char('?') => app.overlay = Some(Overlay::Help { scroll: 0 }),
        KeyCode::Char(':') => app.overlay = Some(Overlay::Palette(PaletteState::new())),
        KeyCode::Char('/') => return run_action(app, ActionId::OpenSearch, rt),
        KeyCode::Char('f') => return run_action(app, ActionId::OpenFilter, rt),
        KeyCode::Char('F') => return run_action(app, ActionId::ClearFilter, rt),
        KeyCode::Char('x') => return run_action(app, ActionId::ClearSearch, rt),
        KeyCode::Char('r') => return run_action(app, ActionId::Refresh, rt),
        KeyCode::Char('R') => return run_action(app, ActionId::ResumeWith, rt),
        KeyCode::Char('i') => return run_action(app, ActionId::OpenImages, rt),
        KeyCode::Char('d') => return run_action(app, ActionId::Delete, rt),
        KeyCode::Char('V') => return run_action(app, ActionId::ClearMarks, rt),
        KeyCode::Tab => return run_action(app, ActionId::ToggleSidebar, rt),
        KeyCode::Char('g' | 'z' | 's') => {
            if let KeyCode::Char(c) = key.code {
                app.pending_prefix = Some(c);
            }
        }
        KeyCode::Char('G') | KeyCode::End => jump_bottom(app, rt),
        KeyCode::Home => jump_top(app, rt),
        KeyCode::PageDown => half_page(app, 2, rt),
        KeyCode::PageUp => half_page(app, -2, rt),
        KeyCode::Char('j') | KeyCode::Down => step(app, 1, rt),
        KeyCode::Char('k') | KeyCode::Up => step(app, -1, rt),
        KeyCode::Char('h') | KeyCode::Left => go_left(app),
        KeyCode::Char('l') | KeyCode::Right => go_right(app, rt),
        KeyCode::Enter => activate(app, rt),
        KeyCode::Char(' ') => toggle_mark(app),
        KeyCode::Esc if app.show_search_results => {
            return run_action(app, ActionId::ClearSearch, rt);
        }
        _ => {}
    }
    Flow::Continue
}

fn step(app: &mut App, delta: i64, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::Sidebar => move_cursor(app, delta, rt),
        Focus::Chat => {
            app.chat_scroll = app.chat_scroll.saturating_add_signed(delta as isize);
        }
    }
}

/// `direction`: ±1 = half page (Ctrl-d/u), ±2 = full page (PgDn/PgUp).
fn half_page(app: &mut App, direction: i64, rt: &tokio::runtime::Runtime) {
    let half = match app.focus {
        Focus::Sidebar => i64::from(app.sidebar_area.height / 2).max(4),
        Focus::Chat => i64::from(app.chat_height / 2).max(4),
    };
    step(app, direction * half, rt);
}

fn move_cursor(app: &mut App, delta: i64, rt: &tokio::runtime::Runtime) {
    if app.rows.is_empty() {
        return;
    }
    let max = app.rows.len() as i64 - 1;
    let new = (app.cursor as i64 + delta).clamp(0, max);
    if new as usize != app.cursor {
        app.cursor = new as usize;
        app.load_messages(rt);
        app.chat_scroll = 0;
    }
}

fn jump_top(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::Sidebar => {
            app.cursor = 0;
            app.load_messages(rt);
        }
        Focus::Chat => app.chat_scroll = 0,
    }
}

fn jump_bottom(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::Sidebar => {
            app.cursor = app.rows.len().saturating_sub(1);
            app.load_messages(rt);
        }
        Focus::Chat => {
            app.chat_scroll = app
                .chat_lines
                .len()
                .saturating_sub(app.chat_height as usize);
        }
    }
}

/// `h`: in chat go back to the sidebar; in the sidebar collapse/jump to group.
fn go_left(app: &mut App) {
    match app.focus {
        Focus::Chat => {
            if app.sidebar_visible {
                app.focus = Focus::Sidebar;
            }
        }
        Focus::Sidebar => match app.cursor_row().cloned() {
            Some(Row::Header {
                key,
                collapsed: false,
                ..
            }) => {
                app.toggle_collapse(&key);
            }
            Some(Row::Session { .. }) => {
                // Jump to the parent header.
                let mut i = app.cursor;
                while i > 0 {
                    i -= 1;
                    if matches!(app.rows.get(i), Some(Row::Header { .. })) {
                        app.cursor = i;
                        break;
                    }
                }
            }
            _ => {}
        },
    }
}

/// `l`: expand a group, or move focus into the chat.
fn go_right(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::Chat => {}
        Focus::Sidebar => match app.cursor_row().cloned() {
            Some(Row::Header {
                key,
                collapsed: true,
                ..
            }) => {
                app.toggle_collapse(&key);
            }
            Some(Row::Session { .. } | Row::Hit { .. }) => {
                app.load_messages(rt);
                app.focus = Focus::Chat;
            }
            _ => {}
        },
    }
}

fn activate(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::Chat => {}
        Focus::Sidebar => match app.cursor_row().cloned() {
            Some(Row::Header { key, .. }) => app.toggle_collapse(&key),
            Some(Row::Session { .. } | Row::Hit { .. }) => {
                app.load_messages(rt);
                app.focus = Focus::Chat;
            }
            None => {}
        },
    }
}

fn toggle_mark(app: &mut App) {
    match app.cursor_row().cloned() {
        Some(Row::Session { .. } | Row::Hit { .. }) => {
            if let Some(id) = app.cursor_conversation_id() {
                if !app.marked.remove(&id) {
                    app.marked.insert(id);
                }
                let max = app.rows.len().saturating_sub(1);
                app.cursor = (app.cursor + 1).min(max);
            }
        }
        Some(Row::Header { .. }) => {
            let ids = app.header_session_ids(app.cursor);
            let all_marked = ids.iter().all(|id| app.marked.contains(id));
            for id in ids {
                if all_marked {
                    app.marked.remove(&id);
                } else {
                    app.marked.insert(id);
                }
            }
        }
        None => {}
    }
}

// =============================================================================
// Actions
// =============================================================================

fn run_action(app: &mut App, action: ActionId, rt: &tokio::runtime::Runtime) -> Flow {
    match action {
        ActionId::Quit => return Flow::Quit,
        ActionId::Help => app.overlay = Some(Overlay::Help { scroll: 0 }),
        ActionId::OpenSearch => {
            app.mode = Mode::Search {
                query: String::new(),
                cursor: 0,
            };
        }
        ActionId::OpenFilter => {
            let query = app.list_filter.clone();
            let cursor = query.chars().count();
            app.mode = Mode::Filter { query, cursor };
        }
        ActionId::ClearFilter => {
            app.list_filter.clear();
            app.rebuild();
            app.set_status("Filter cleared");
        }
        ActionId::ClearSearch => {
            app.clear_search();
            app.set_status("Search cleared");
        }
        ActionId::Refresh => app.refresh_data(rt),
        ActionId::ToggleSidebar => {
            app.sidebar_visible = !app.sidebar_visible;
            if !app.sidebar_visible {
                app.focus = Focus::Chat;
            } else {
                app.focus = Focus::Sidebar;
            }
        }
        ActionId::GroupFlat => set_group(app, GroupBy::Flat),
        ActionId::GroupAgent => set_group(app, GroupBy::Agent),
        ActionId::GroupRepo => set_group(app, GroupBy::Repo),
        ActionId::GroupAgentRepo => set_group(app, GroupBy::AgentRepo),
        ActionId::GroupDate => set_group(app, GroupBy::Date),
        ActionId::CollapseAll => {
            let keys = app.all_group_keys();
            app.collapsed.extend(keys);
            app.rebuild();
        }
        ActionId::ExpandAll => {
            app.collapsed.clear();
            app.rebuild();
        }
        ActionId::SortDateDesc => set_sort(app, SortOrder::DateDesc),
        ActionId::SortDateAsc => set_sort(app, SortOrder::DateAsc),
        ActionId::SortTitle => set_sort(app, SortOrder::TitleAsc),
        ActionId::MarkAll => {
            let ids: Vec<Uuid> = app
                .visible
                .iter()
                .map(|&i| app.all_conversations[i].id)
                .collect();
            app.marked.extend(ids);
        }
        ActionId::ClearMarks => {
            app.marked.clear();
        }
        ActionId::Delete => {
            if let Some(Row::Header {
                source_id: Some(source_id),
                label,
                ..
            }) = app.cursor_row().cloned()
                && app.marked.is_empty()
            {
                app.overlay = Some(Overlay::ConfirmDeleteSource {
                    source_id,
                    name: label,
                });
            } else {
                let ids = app.delete_targets();
                if !ids.is_empty() {
                    app.overlay = Some(Overlay::ConfirmDelete { ids });
                }
            }
        }
        ActionId::Resume => {
            if let Some(id) = app.cursor_conversation_id() {
                app.pending_resume = Some((id, None));
                return Flow::Quit;
            }
            app.set_status("No session selected");
        }
        ActionId::ResumeWith => {
            if let Some(id) = app.cursor_conversation_id() {
                let mut agents: Vec<String> = app.config.resume.agents.keys().cloned().collect();
                agents.sort();
                if agents.is_empty() {
                    app.set_status("No [resume.agents] configured");
                } else {
                    let cursor = agents
                        .iter()
                        .position(|a| *a == app.config.resume.default_agent)
                        .unwrap_or(0);
                    app.overlay = Some(Overlay::AgentPicker {
                        conv: id,
                        agents,
                        cursor,
                    });
                }
            } else {
                app.set_status("No session selected");
            }
        }
        ActionId::OpenImages => {
            if app.chat_images.is_empty() {
                app.set_status("No images in this conversation");
            } else {
                let viewer = ViewerState::open(&app.chat_images, 0, &app.image_picker, &app.db, rt);
                app.overlay = Some(Overlay::ImageViewer(Box::new(viewer)));
            }
        }
    }
    Flow::Continue
}

fn set_group(app: &mut App, group: GroupBy) {
    app.group_by = group;
    app.rebuild();
    app.set_status(format!("Grouped by {}", group.label()));
}

fn set_sort(app: &mut App, sort: SortOrder) {
    app.sort_order = sort;
    app.rebuild();
    app.set_status(format!("Sorted by {}", sort.label()));
}

// =============================================================================
// Text-entry modes
// =============================================================================

fn handle_search_input(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) {
    let Mode::Search { query, cursor } = &mut app.mode else {
        return;
    };
    match key.code {
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Enter => {
            let q = query.clone();
            app.mode = Mode::Normal;
            app.perform_search(&q, rt);
            app.load_messages(rt);
        }
        KeyCode::Tab => {
            app.search_scope = app.search_scope.next();
        }
        KeyCode::Backspace if *cursor > 0 => {
            *cursor -= 1;
            remove_char(query, *cursor);
        }
        KeyCode::Delete => {
            remove_char(query, *cursor);
        }
        KeyCode::Left => *cursor = cursor.saturating_sub(1),
        KeyCode::Right => *cursor = (*cursor + 1).min(query.chars().count()),
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = query.chars().count(),
        KeyCode::Char(c) => {
            insert_char(query, *cursor, c);
            *cursor += 1;
        }
        _ => {}
    }
}

fn handle_filter_input(app: &mut App, key: &KeyEvent) {
    let Mode::Filter { query, cursor } = &mut app.mode else {
        return;
    };
    let mut changed = false;
    match key.code {
        KeyCode::Esc | KeyCode::Enter => {
            app.mode = Mode::Normal;
            return;
        }
        KeyCode::Backspace if *cursor > 0 => {
            *cursor -= 1;
            remove_char(query, *cursor);
            changed = true;
        }
        KeyCode::Delete => {
            remove_char(query, *cursor);
            changed = true;
        }
        KeyCode::Left => *cursor = cursor.saturating_sub(1),
        KeyCode::Right => *cursor = (*cursor + 1).min(query.chars().count()),
        KeyCode::Char(c) => {
            insert_char(query, *cursor, c);
            *cursor += 1;
            changed = true;
        }
        _ => {}
    }
    if changed {
        app.list_filter = query.clone();
        app.rebuild();
    }
}

fn insert_char(s: &mut String, char_idx: usize, c: char) {
    let byte_idx = s.char_indices().nth(char_idx).map_or(s.len(), |(i, _)| i);
    s.insert(byte_idx, c);
}

fn remove_char(s: &mut String, char_idx: usize) {
    if let Some((byte_idx, _)) = s.char_indices().nth(char_idx) {
        s.remove(byte_idx);
    }
}

// =============================================================================
// Overlays
// =============================================================================

fn handle_overlay(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) -> Flow {
    let Some(overlay) = app.overlay.take() else {
        return Flow::Continue;
    };

    match overlay {
        Overlay::Help { mut scroll } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q' | '?') => return Flow::Continue,
                KeyCode::Char('j') | KeyCode::Down => scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => scroll += 10,
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                _ => {}
            }
            app.overlay = Some(Overlay::Help { scroll });
        }
        Overlay::Palette(mut palette) => match key.code {
            KeyCode::Esc => {}
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            KeyCode::Enter => {
                if let Some(action) = palette.selected() {
                    return run_action(app, action, rt);
                }
            }
            KeyCode::Down => {
                palette.cursor = (palette.cursor + 1).min(palette.matches.len().saturating_sub(1));
                app.overlay = Some(Overlay::Palette(palette));
            }
            KeyCode::Up => {
                palette.cursor = palette.cursor.saturating_sub(1);
                app.overlay = Some(Overlay::Palette(palette));
            }
            KeyCode::Backspace => {
                palette.query.pop();
                palette.refilter();
                app.overlay = Some(Overlay::Palette(palette));
            }
            KeyCode::Char(c) => {
                palette.query.push(c);
                palette.refilter();
                app.overlay = Some(Overlay::Palette(palette));
            }
            _ => app.overlay = Some(Overlay::Palette(palette)),
        },
        Overlay::ConfirmDelete { ids } => match key.code {
            KeyCode::Char('y') => {
                let count = ids.len();
                match rt.block_on(app.db.delete_conversations_batch(&ids)) {
                    Ok(deleted) => {
                        app.marked.clear();
                        app.loaded_conversation = None;
                        app.refresh_data(rt);
                        app.set_status(format!("Deleted {deleted}/{count}"));
                    }
                    Err(e) => app.set_status(format!("Delete error: {e}")),
                }
            }
            KeyCode::Esc | KeyCode::Char('n') => {}
            _ => app.overlay = Some(Overlay::ConfirmDelete { ids }),
        },
        Overlay::ConfirmDeleteSource { source_id, name } => match key.code {
            KeyCode::Char('y') => match rt.block_on(app.db.remove_source(&source_id)) {
                Ok(()) => {
                    app.loaded_conversation = None;
                    app.refresh_data(rt);
                    app.set_status(format!("Deleted source '{name}'"));
                }
                Err(e) => app.set_status(format!("Error deleting source: {e}")),
            },
            KeyCode::Esc | KeyCode::Char('n') => {}
            _ => app.overlay = Some(Overlay::ConfirmDeleteSource { source_id, name }),
        },
        Overlay::AgentPicker {
            conv,
            agents,
            mut cursor,
        } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Enter => {
                let agent = agents.get(cursor).cloned();
                app.pending_resume = Some((conv, agent));
                return Flow::Quit;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                cursor = (cursor + 1).min(agents.len().saturating_sub(1));
                app.overlay = Some(Overlay::AgentPicker {
                    conv,
                    agents,
                    cursor,
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                cursor = cursor.saturating_sub(1);
                app.overlay = Some(Overlay::AgentPicker {
                    conv,
                    agents,
                    cursor,
                });
            }
            _ => {
                app.overlay = Some(Overlay::AgentPicker {
                    conv,
                    agents,
                    cursor,
                });
            }
        },
        Overlay::ImageViewer(mut viewer) => match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'i') => {}
            KeyCode::Char('n' | 'j' | 'l') | KeyCode::Right | KeyCode::Down => {
                viewer.next(&app.chat_images, &app.image_picker, &app.db, rt);
                app.overlay = Some(Overlay::ImageViewer(viewer));
            }
            KeyCode::Char('p' | 'k' | 'h') | KeyCode::Left | KeyCode::Up => {
                viewer.previous(&app.chat_images, &app.image_picker, &app.db, rt);
                app.overlay = Some(Overlay::ImageViewer(viewer));
            }
            _ => app.overlay = Some(Overlay::ImageViewer(viewer)),
        },
    }
    Flow::Continue
}
