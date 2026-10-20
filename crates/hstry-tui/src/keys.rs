//! Input handling: query typing, normal-mode keys, overlays, and the actions
//! they all dispatch to.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use uuid::Uuid;

use crate::actions::{ActionId, PaletteState, resolve_progression};
use crate::clipboard;
use crate::images::ViewerState;
use crate::state::{App, ExportFormat, Focus, GroupBy, Mode, Overlay, Row, Scope, SortOrder};

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub fn handle_key(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) -> Flow {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Flow::Quit;
    }
    if app.overlay.is_some() {
        return handle_overlay(app, key, rt);
    }
    match app.mode {
        Mode::Normal => handle_normal(app, key, rt),
        Mode::Query => {
            handle_query(app, key, rt);
            Flow::Continue
        }
    }
}

pub fn handle_mouse(
    app: &mut App,
    kind: MouseEventKind,
    col: u16,
    row: u16,
    rt: &tokio::runtime::Runtime,
) {
    let pos = ratatui::layout::Position::new(col, row);
    let in_chat = app.chat_area.contains(pos);
    match kind {
        MouseEventKind::ScrollDown if in_chat => {
            app.chat_scroll = app.chat_scroll.saturating_add(3)
        }
        MouseEventKind::ScrollUp if in_chat => app.chat_scroll = app.chat_scroll.saturating_sub(3),
        MouseEventKind::ScrollDown => move_cursor(app, 1, rt),
        MouseEventKind::ScrollUp => move_cursor(app, -1, rt),
        MouseEventKind::Down(_) if in_chat => app.focus = Focus::Transcript,
        MouseEventKind::Down(_) if app.list_area.contains(pos) => app.focus = Focus::List,
        _ => {}
    }
}

// =============================================================================
// Query bar
// =============================================================================

fn handle_query(app: &mut App, key: &KeyEvent, rt: &tokio::runtime::Runtime) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.clear_query();
            app.load_messages(rt);
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            app.focus = Focus::List;
            // Run a pending content search now instead of waiting.
            if app.scope == Scope::Content && app.query_changed_at.is_some() {
                app.query_changed_at =
                    Some(std::time::Instant::now() - crate::state::CONTENT_SEARCH_DEBOUNCE);
                app.tick(rt);
            }
            app.load_messages(rt);
        }
        KeyCode::Tab | KeyCode::BackTab => {
            app.set_scope(app.scope.toggle());
            app.load_messages(rt);
        }
        KeyCode::Down => move_cursor(app, 1, rt),
        KeyCode::Up => move_cursor(app, -1, rt),
        KeyCode::Char('n' | 'j') if ctrl => move_cursor(app, 1, rt),
        KeyCode::Char('p' | 'k') if ctrl => move_cursor(app, -1, rt),
        KeyCode::PageDown => move_cursor(app, 8, rt),
        KeyCode::PageUp => move_cursor(app, -8, rt),
        KeyCode::Char('u') if ctrl => {
            app.query.clear();
            app.query_cursor = 0;
            app.query_edited();
            app.load_messages(rt);
        }
        KeyCode::Char('w') if ctrl => {
            let chars: Vec<char> = app.query.chars().collect();
            let mut i = app.query_cursor;
            while i > 0 && chars[i - 1] == ' ' {
                i -= 1;
            }
            while i > 0 && chars[i - 1] != ' ' {
                i -= 1;
            }
            let rest: String = chars[app.query_cursor..].iter().collect();
            app.query = chars[..i].iter().collect::<String>() + &rest;
            app.query_cursor = i;
            app.query_edited();
            app.load_messages(rt);
        }
        KeyCode::Backspace if app.query_cursor > 0 => {
            app.query_cursor -= 1;
            remove_char(&mut app.query, app.query_cursor);
            app.query_edited();
            app.load_messages(rt);
        }
        KeyCode::Delete => {
            remove_char(&mut app.query, app.query_cursor);
            app.query_edited();
            app.load_messages(rt);
        }
        KeyCode::Left => app.query_cursor = app.query_cursor.saturating_sub(1),
        KeyCode::Right => app.query_cursor = (app.query_cursor + 1).min(app.query.chars().count()),
        KeyCode::Home => app.query_cursor = 0,
        KeyCode::End => app.query_cursor = app.query.chars().count(),
        KeyCode::Char(c) if !ctrl => {
            insert_char(&mut app.query, app.query_cursor, c);
            app.query_cursor += 1;
            app.query_edited();
            app.load_messages(rt);
        }
        _ => {}
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
        KeyCode::Char('p') if ctrl => app.overlay = Some(Overlay::Palette(PaletteState::new())),
        KeyCode::Char('r') if ctrl => return run_action(app, ActionId::OpenDefault, rt),
        KeyCode::Char('a') if ctrl => return run_action(app, ActionId::MarkAll, rt),
        KeyCode::Char('d') if ctrl => half_page(app, 1, rt),
        KeyCode::Char('u') if ctrl => half_page(app, -1, rt),
        KeyCode::Char('f') if ctrl => return run_action(app, ActionId::Find, rt),
        KeyCode::Char('q') => return Flow::Quit,
        KeyCode::Char('?') => app.overlay = Some(Overlay::Help { scroll: 0 }),
        KeyCode::Char(':') => app.overlay = Some(Overlay::Palette(PaletteState::new())),
        KeyCode::Char('/' | 'f') => return run_action(app, ActionId::Find, rt),
        KeyCode::Char('x' | 'F') => return run_action(app, ActionId::ClearQuery, rt),
        KeyCode::Char('r') => return run_action(app, ActionId::Refresh, rt),
        KeyCode::Char('o' | 'R') => return run_action(app, ActionId::OpenWith, rt),
        KeyCode::Char('O') => return run_action(app, ActionId::OpenDefault, rt),
        KeyCode::Char('i') => return run_action(app, ActionId::OpenImages, rt),
        KeyCode::Char('y') => return run_action(app, ActionId::CopySession, rt),
        KeyCode::Char('Y') => return run_action(app, ActionId::CopyId, rt),
        KeyCode::Char('p') => return run_action(app, ActionId::CopyPath, rt),
        KeyCode::Char('e') => return run_action(app, ActionId::Export, rt),
        KeyCode::Char('t') => return run_action(app, ActionId::ToggleDetails, rt),
        KeyCode::Char('d') => return run_action(app, ActionId::Delete, rt),
        KeyCode::Char('V') => return run_action(app, ActionId::ClearMarks, rt),
        KeyCode::Tab | KeyCode::BackTab => return run_action(app, ActionId::SwitchPane, rt),
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
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => go_right(app, rt),
        KeyCode::Char(' ') => toggle_mark(app),
        KeyCode::Esc => {
            if app.focus == Focus::Transcript {
                app.focus = Focus::List;
            } else if !app.query.is_empty() {
                return run_action(app, ActionId::ClearQuery, rt);
            } else if !app.marked.is_empty() {
                app.marked.clear();
            }
        }
        _ => {}
    }
    Flow::Continue
}

fn step(app: &mut App, delta: i64, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::List => move_cursor(app, delta, rt),
        Focus::Transcript => {
            app.chat_scroll = app.chat_scroll.saturating_add_signed(delta as isize);
        }
    }
}

/// `direction`: ±1 = half page (Ctrl-d/u), ±2 = full page (PgDn/PgUp).
fn half_page(app: &mut App, direction: i64, rt: &tokio::runtime::Runtime) {
    match app.focus {
        // List rows are two lines tall.
        Focus::List => {
            let rows = i64::from(app.list_area.height.saturating_sub(2) / 4).max(3);
            move_cursor(app, direction * rows, rt);
        }
        Focus::Transcript => {
            let half = i64::from(app.chat_height / 2).max(4);
            step(app, direction * half, rt);
        }
    }
}

pub fn move_cursor(app: &mut App, delta: i64, rt: &tokio::runtime::Runtime) {
    if app.rows.is_empty() {
        return;
    }
    let max = app.rows.len() as i64 - 1;
    let new = (app.cursor as i64 + delta).clamp(0, max);
    if new as usize != app.cursor {
        app.cursor = new as usize;
        app.load_messages(rt);
    }
}

fn jump_top(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::List => {
            app.cursor = 0;
            app.load_messages(rt);
        }
        Focus::Transcript => app.chat_scroll = 0,
    }
}

fn jump_bottom(app: &mut App, rt: &tokio::runtime::Runtime) {
    match app.focus {
        Focus::List => {
            app.cursor = app.rows.len().saturating_sub(1);
            app.load_messages(rt);
        }
        Focus::Transcript => {
            app.chat_scroll = app
                .chat_lines
                .len()
                .saturating_sub(app.chat_height as usize);
        }
    }
}

/// `h`: back to the list; in the list collapse a group / jump to its header.
fn go_left(app: &mut App) {
    match app.focus {
        Focus::Transcript => app.focus = Focus::List,
        Focus::List => match app.cursor_row().cloned() {
            Some(Row::Header {
                key,
                collapsed: false,
                ..
            }) => app.toggle_collapse(&key),
            Some(Row::Session { depth, .. }) if depth > 0 => {
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

/// `l`/Enter: expand a group, or read the selected session.
fn go_right(app: &mut App, rt: &tokio::runtime::Runtime) {
    if app.focus == Focus::Transcript {
        return;
    }
    match app.cursor_row().cloned() {
        Some(Row::Header { key, .. }) => app.toggle_collapse(&key),
        Some(Row::Session { .. } | Row::Hits { .. }) => {
            app.load_messages(rt);
            app.focus = Focus::Transcript;
        }
        None => {}
    }
}

fn toggle_mark(app: &mut App) {
    match app.cursor_row().cloned() {
        Some(Row::Session { .. } | Row::Hits { .. }) => {
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

pub fn run_action(app: &mut App, action: ActionId, rt: &tokio::runtime::Runtime) -> Flow {
    match action {
        ActionId::Quit => return Flow::Quit,
        ActionId::Help => app.overlay = Some(Overlay::Help { scroll: 0 }),
        ActionId::Find => {
            app.mode = Mode::Query;
            app.query_cursor = app.query.chars().count();
            app.focus = Focus::List;
        }
        ActionId::FindContent => {
            app.mode = Mode::Query;
            app.query_cursor = app.query.chars().count();
            app.focus = Focus::List;
            app.set_scope(Scope::Content);
        }
        ActionId::ClearQuery => {
            app.clear_query();
            app.load_messages(rt);
        }
        ActionId::Refresh => app.refresh_data(rt),
        ActionId::SwitchPane => {
            app.focus = match app.focus {
                Focus::List if app.cursor_conversation_id().is_some() => {
                    app.load_messages(rt);
                    Focus::Transcript
                }
                _ => Focus::List,
            };
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
                .map(|v| app.sessions[v.idx].conv.id)
                .collect();
            let n = ids.len();
            app.marked.extend(ids);
            app.set_status(format!("Marked {n} sessions"));
        }
        ActionId::ClearMarks => app.marked.clear(),
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
        ActionId::OpenDefault => {
            if let Some(id) = app.cursor_conversation_id() {
                app.pending_resume = Some((id, None));
                return Flow::Quit;
            }
            app.set_status("No session selected");
        }
        ActionId::OpenWith => {
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
        ActionId::CopySession => {
            if app.cursor_conversation_id().is_none() {
                app.set_status("No session selected");
            } else {
                app.load_messages(rt);
                let conv = app.cursor_session().map(|s| &s.conv);
                let md = clipboard::session_markdown(conv, &app.messages);
                match clipboard::copy_text(&md) {
                    Ok(target) => app.set_status(format!(
                        "Copied {} messages to the {target}",
                        app.messages.len()
                    )),
                    Err(e) => app.set_status(format!("Copy failed: {e}")),
                }
            }
        }
        ActionId::CopyId => match app.cursor_conversation_id() {
            Some(id) => copy_and_report(app, &id.to_string(), "session id"),
            None => app.set_status("No session selected"),
        },
        ActionId::CopyPath => {
            let path = app.cursor_session().and_then(|s| {
                s.file_path().or_else(|| {
                    app.sources
                        .iter()
                        .find(|src| src.id == s.conv.source_id)
                        .and_then(|src| src.path.clone())
                })
            });
            match path {
                Some(path) => copy_and_report(app, &path, "path"),
                None => app.set_status("No on-disk path recorded for this session"),
            }
        }
        ActionId::Export => match app.cursor_conversation_id() {
            Some(conv) => app.overlay = Some(Overlay::ExportPicker { conv, cursor: 0 }),
            None => app.set_status("No session selected"),
        },
        ActionId::ToggleDetails => {
            app.expand_details = !app.expand_details;
            app.rebuild_chat_lines();
            app.set_status(if app.expand_details {
                "Showing full thinking and tool output"
            } else {
                "Collapsed thinking and tool output"
            });
        }
        ActionId::OpenImages => {
            if app.chat_images.is_empty() {
                app.set_status("No images in this session");
            } else {
                let viewer = ViewerState::open(&app.chat_images, 0, &app.image_picker, &app.db, rt);
                app.overlay = Some(Overlay::ImageViewer(Box::new(viewer)));
            }
        }
    }
    Flow::Continue
}

fn copy_and_report(app: &mut App, text: &str, what: &str) {
    match clipboard::copy_text(text) {
        Ok(_) => app.set_status(format!(
            "Copied {what}: {}",
            crate::markdown::truncate_str(text, 60)
        )),
        Err(e) => app.set_status(format!("Copy failed: {e}")),
    }
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

/// Write the session in `format` to the current directory.
fn export_session(app: &mut App, conv: Uuid, format: ExportFormat, rt: &tokio::runtime::Runtime) {
    let messages = if app.loaded_conversation == Some(conv) {
        app.messages.clone()
    } else {
        match rt.block_on(app.db.get_messages(conv)) {
            Ok(m) => m,
            Err(e) => {
                app.set_status(format!("Export failed: {e}"));
                return;
            }
        }
    };
    let Some(session) = app.session_by_id(conv) else {
        app.set_status("Session not found");
        return;
    };
    let body = match format {
        ExportFormat::Markdown => clipboard::session_markdown(Some(&session.conv), &messages),
        ExportFormat::Json => match serde_json::to_string_pretty(&serde_json::json!({
            "conversation": session.conv,
            "messages": messages,
        })) {
            Ok(s) => s,
            Err(e) => {
                app.set_status(format!("Export failed: {e}"));
                return;
            }
        },
    };
    let name = format!(
        "hstry-{}-{}.{}",
        slug(session.title()),
        &conv.to_string()[..8],
        format.extension()
    );
    let path = std::env::current_dir()
        .map(|d| d.join(&name))
        .unwrap_or_else(|_| name.clone().into());
    match std::fs::write(&path, body) {
        Ok(()) => app.set_status(format!("Exported to {}", path.display())),
        Err(e) => app.set_status(format!("Export failed: {e}")),
    }
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
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
            code => {
                match code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        cursor = (cursor + 1).min(agents.len().saturating_sub(1));
                    }
                    KeyCode::Char('k') | KeyCode::Up => cursor = cursor.saturating_sub(1),
                    _ => {}
                }
                app.overlay = Some(Overlay::AgentPicker {
                    conv,
                    agents,
                    cursor,
                });
            }
        },
        Overlay::ExportPicker { conv, mut cursor } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Enter => {
                if let Some(format) = ExportFormat::ALL.get(cursor).copied() {
                    export_session(app, conv, format, rt);
                }
            }
            code => {
                match code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        cursor = (cursor + 1).min(ExportFormat::ALL.len() - 1);
                    }
                    KeyCode::Char('k') | KeyCode::Up => cursor = cursor.saturating_sub(1),
                    _ => {}
                }
                app.overlay = Some(Overlay::ExportPicker { conv, cursor });
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

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(
            slug("Fix flaky retry test: in the sync loop!"),
            "fix-flaky-retry-test-in-the-sync-loop"
        );
        assert_eq!(slug("¿¿??"), "session");
    }
}
