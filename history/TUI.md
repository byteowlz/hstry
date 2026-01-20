# TUI Architecture Guide

This document describes the abstracted TUI architecture patterns used in this project. These patterns can be reused across different applications.

## Overview

The TUI follows a three-pane layout with vim-style navigation, a modal system for different interaction contexts, and a WhichKey-inspired command discovery system.

## Layout Structure

### Three-Pane Layout

The main interface is divided into three horizontal panes:

```
+---------------+------------------+------------------+
|               |                  |                  |
|   Left Pane   |   Middle Pane    |   Right Pane     |
|     (20%)     |      (40%)       |      (40%)       |
|               |                  |                  |
|  Navigation/  |   Item List      |   Preview/       |
|  Filtering    |                  |   Details        |
|               |                  |                  |
+---------------+------------------+------------------+
|                    Status Bar                       |
+-----------------------------------------------------+
```

**Layout Constraints:**
- Vertical split: Main area (Min) + Status bar (Length 1)
- Horizontal split: Left (20%) | Middle (40%) | Right (40%)

**Pane Purposes:**
- **Left Pane**: Navigation tree, filters, categories, or view switching
- **Middle Pane**: Primary list of items with multi-selection support
- **Right Pane**: Detail view, preview, or alternative representations

### Status Bar

A single-line status bar at the bottom displays:
- Current mode indicator (color-coded)
- Context information (e.g., current store/scope)
- Item counts
- Ephemeral status messages
- Quick help hints

## Modal System

### App Modes

The application uses a mode-based interaction system:

```
Normal          - Default browsing mode
Search          - Text input for searching with query buffer
Delete          - Confirmation for single item deletion
DeleteMultiple  - Confirmation for batch deletion
Help            - Scrollable help overlay
Sort            - Sort option selection menu
WhichKey        - Command discovery submenu
CategoryInput   - Text input for new values
CategorySelect  - Selection from existing options
StoreSelect     - Context/scope switching
StoreCreate     - Create new context/scope
MoveToStore     - Move item between contexts
Export          - Export options dialog
```

**Mode Transitions:**
- Most modes return to Normal on Escape or completion
- WhichKey modes can chain to input/select modes
- Delete modes require explicit confirmation (y/n)

## WhichKey Implementation

The WhichKey system provides contextual command discovery through a bottom-anchored overlay bar.

### Structure

```
+-----------------------------------------------------+
|  [Context] [key1] Action1 | [key2] Action2 | ...    |
+-----------------------------------------------------+
```

### WhichKey Contexts

Each context defines available sub-commands:

```
Type Context:
  [e] Episodic    [s] Semantic    [p] Procedural

Importance Context:
  [0-9] Set       [i] Increase    [d] Decrease

Category Context:
  [n] New         [s] Select
```

### Implementation Pattern

1. **Activation**: Single key press in Normal mode (e.g., `t`, `i`, `c`)
2. **Display**: Render options bar at screen bottom
3. **Selection**: Single key press selects option or chains to another mode
4. **Exit**: Escape returns to Normal, selection triggers action and returns

### Code Pattern

```rust
enum WhichKeyContext {
    Type,       // Memory type selection
    Importance, // Importance value
    Category,   // Category management
}

// Rendering pattern
fn draw(context: &WhichKeyContext) {
    let (title, items) = match context {
        WhichKeyContext::Type => (
            "Type",
            vec![("e", "Episodic"), ("s", "Semantic"), ("p", "Procedural")],
        ),
        // ...
    };
    // Render [key] Label pairs in a bottom bar
}
```

## Shortcut Combinations

### Vim-Style Navigation

**Basic Movement:**
| Key | Action |
|-----|--------|
| `j` / Down | Move down in list |
| `k` / Up | Move up in list |
| `h` / Left | Switch to left pane |
| `l` / Right | Switch to right pane |
| `gg` | Jump to top (double-tap detection) |
| `G` | Jump to bottom |
| `Ctrl-d` | Page down |
| `Ctrl-u` | Page up |

### G-Prefix Pattern

The `g` key acts as a prefix for extended commands:

```rust
// State tracking
g_prefix: bool

// In key handler
KeyAction::Char('g') => {
    if self.g_prefix {
        self.move_top();      // gg = go to top
        self.g_prefix = false;
    } else {
        self.g_prefix = true; // Wait for second key
    }
}
```

Any non-g key after `g` prefix resets the prefix state.

### Selection System

| Key | Action |
|-----|--------|
| `Space` | Toggle selection on current item + move down |
| `Ctrl-a` | Select all items |
| `V` | Clear all selections |

Selection state is maintained separately from cursor position using a `HashSet<usize>`.

### Quick Actions

| Key | Action | Notes |
|-----|--------|-------|
| `d` | Delete | Respects multi-selection |
| `e` | Edit | Opens external editor |
| `a` | Add | Create new item |
| `r` | Refresh | Reload data |
| `/` or `:` | Search | Opens command palette |
| `s` | Sort | Opens sort menu |
| `v` | Toggle view | Cycles right pane view mode |
| `b` | Browse | Cycles through data views |

### WhichKey Triggers

| Key | Context | Sub-options |
|-----|---------|-------------|
| `t` | Type | `e`/`s`/`p` |
| `i` | Importance | `0-9`/`i`/`d` |
| `c` | Category | `n`/`s` |

### Context Operations

| Key | Action |
|-----|--------|
| `S` | Switch context/store |
| `m` | Move to context |
| `E` | Export |

## Event Handling

### Key Action Parsing

Keys are parsed into semantic actions:

```rust
enum KeyAction {
    Quit,           // Ctrl-c
    Up, Down,       // Arrow keys
    Left, Right,    // Arrow keys
    PageDown,       // Ctrl-d
    PageUp,         // Ctrl-u
    Select,         // Enter
    Escape,         // Esc
    Backspace,      // Backspace
    Char(char),     // Any printable character
    ToggleSelect,   // Space
    SelectAll,      // Ctrl-a
    CycleSearchMode,// Tab
    Noop,           // Unknown/ignored
}
```

### Mode-Specific Handling

Each mode has its own key handler:

```rust
async fn handle_key_action(&mut self, action: KeyAction) -> Result<bool> {
    match self.mode {
        AppMode::Normal => self.handle_normal_mode(action).await,
        AppMode::Search(_) => self.handle_search_mode(action).await,
        AppMode::Delete(_) => self.handle_delete_mode(action).await,
        // ...
    }
}
```

## Overlay/Popup System

### Centered Popup Pattern

Modal dialogs use a centered popup function:

```rust
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    // Vertical centering
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    // Horizontal centering
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

### Overlay Types

- **Confirmation dialogs**: 50x20%, destructive action confirmation
- **Sort menu**: 40x30%, numbered list selection
- **Help screen**: 70x90%, scrollable documentation
- **Command palette**: 80x20%, search input with mode indicator
- **Selection lists**: 50-60x40-50%, item picker with current indicator

### Rendering Order

1. Render base three-pane layout
2. Render status bar
3. If modal mode active, render overlay on top

```rust
pub fn draw(f: &mut Frame, app: &App) {
    // Base layout
    left_pane::draw(f, app, main_chunks[0]);
    middle_pane::draw(f, app, main_chunks[1]);
    right_pane::draw(f, app, main_chunks[2]);
    status_bar::draw(f, app, status_area);

    // Modal overlays
    match &app.mode {
        AppMode::Help => help::draw(f, app),
        AppMode::Sort => draw_sort_menu(f, app),
        AppMode::Search(_) => command_palette::draw(f, app),
        AppMode::WhichKey(ctx) => whichkey::draw(f, ctx),
        // ...
    }
}
```

## Selection State Management

### Selection Structure

```rust
struct Selection {
    index: usize,               // Current cursor position
    offset: usize,              // Viewport scroll offset
    selected_indices: HashSet<usize>,  // Multi-selected items
}
```

### Navigation Methods

- `next(max, page_size)`: Move down, adjust offset if cursor exceeds viewport
- `previous()`: Move up, adjust offset if cursor goes above viewport
- `top()`: Jump to index 0, reset offset
- `bottom(max, page_size)`: Jump to last item, adjust offset
- `page_down/up(max, page_size)`: Move by page_size

### Selection Methods

- `toggle_selection()`: Add/remove current index from selection
- `select_all(max)`: Select all indices 0..max
- `deselect_all()`: Clear selection set
- `has_selections()`: Check if any items selected
- `get_selected_indices()`: Get sorted list of selected indices

## Filter State

The left pane often serves as a filter panel:

```rust
struct FilterState {
    enabled_categories: HashSet<String>,
    enabled_types: HashSet<Type>,
    enabled_tags: HashSet<String>,
    show_recent: bool,
    show_important: bool,
}
```

Filter patterns:
- **Toggle**: Enable/disable individual filter
- **Isolate**: Enable only the selected filter, disable all others
- **Clear**: Reset all filters to default (show all)

## Best Practices

1. **Mode isolation**: Each mode handles its own keys, preventing key conflicts
2. **Escape always exits**: Every mode should return to Normal on Escape
3. **Visual feedback**: Mode changes should update status bar immediately
4. **Consistent navigation**: vim keys work in all scrollable contexts
5. **Confirmation for destructive actions**: Delete requires explicit `y` confirmation
6. **Context persistence**: Remember last position when switching views
7. **Status messages**: Show ephemeral feedback for completed actions
