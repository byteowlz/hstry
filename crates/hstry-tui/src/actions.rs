//! Actions are data: one table, surfaced via keys, key progressions, and the
//! command palette.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    Quit,
    Help,
    Find,
    FindContent,
    ClearQuery,
    Refresh,
    SwitchPane,
    GroupFlat,
    GroupAgent,
    GroupRepo,
    GroupAgentRepo,
    GroupDate,
    CollapseAll,
    ExpandAll,
    SortDateDesc,
    SortDateAsc,
    SortTitle,
    Delete,
    OpenDefault,
    OpenWith,
    OpenImages,
    CopySession,
    CopyId,
    CopyPath,
    Export,
    ToggleDetails,
    MarkAll,
    ClearMarks,
}

pub struct Action {
    pub id: ActionId,
    pub label: &'static str,
    /// Key path, for display in palette and whichkey hints.
    pub keys: &'static str,
}

pub const ACTIONS: &[Action] = &[
    Action {
        id: ActionId::Find,
        label: "Find sessions (title, repo, agent, first prompt)",
        keys: "/",
    },
    Action {
        id: ActionId::FindContent,
        label: "Search inside every message",
        keys: "/ then Tab",
    },
    Action {
        id: ActionId::OpenWith,
        label: "Open session in agent…",
        keys: "o",
    },
    Action {
        id: ActionId::OpenDefault,
        label: "Open session in default agent",
        keys: "O",
    },
    Action {
        id: ActionId::CopySession,
        label: "Copy transcript (markdown)",
        keys: "y",
    },
    Action {
        id: ActionId::CopyId,
        label: "Copy session id",
        keys: "Y",
    },
    Action {
        id: ActionId::CopyPath,
        label: "Copy session file path",
        keys: "p",
    },
    Action {
        id: ActionId::Export,
        label: "Export session to a file…",
        keys: "e",
    },
    Action {
        id: ActionId::ToggleDetails,
        label: "Expand / collapse thinking and tool output",
        keys: "t",
    },
    Action {
        id: ActionId::OpenImages,
        label: "View images in session",
        keys: "i",
    },
    Action {
        id: ActionId::ClearQuery,
        label: "Clear the query",
        keys: "x",
    },
    Action {
        id: ActionId::SwitchPane,
        label: "Switch between list and transcript",
        keys: "Tab",
    },
    Action {
        id: ActionId::GroupFlat,
        label: "List: flat, no groups",
        keys: "z f",
    },
    Action {
        id: ActionId::GroupAgent,
        label: "Group by agent",
        keys: "z a",
    },
    Action {
        id: ActionId::GroupRepo,
        label: "Group by repo",
        keys: "z r",
    },
    Action {
        id: ActionId::GroupAgentRepo,
        label: "Group by agent then repo",
        keys: "z A",
    },
    Action {
        id: ActionId::GroupDate,
        label: "Group by month",
        keys: "z d",
    },
    Action {
        id: ActionId::CollapseAll,
        label: "Collapse all groups",
        keys: "z c",
    },
    Action {
        id: ActionId::ExpandAll,
        label: "Expand all groups",
        keys: "z o",
    },
    Action {
        id: ActionId::SortDateDesc,
        label: "Sort by last activity (newest first)",
        keys: "s d",
    },
    Action {
        id: ActionId::SortDateAsc,
        label: "Sort by last activity (oldest first)",
        keys: "s D",
    },
    Action {
        id: ActionId::SortTitle,
        label: "Sort by title",
        keys: "s t",
    },
    Action {
        id: ActionId::MarkAll,
        label: "Mark all visible sessions",
        keys: "Ctrl-A",
    },
    Action {
        id: ActionId::ClearMarks,
        label: "Clear marks",
        keys: "V",
    },
    Action {
        id: ActionId::Delete,
        label: "Delete marked / selected session",
        keys: "d",
    },
    Action {
        id: ActionId::Refresh,
        label: "Reload sessions from the database",
        keys: "r",
    },
    Action {
        id: ActionId::Help,
        label: "Help",
        keys: "?",
    },
    Action {
        id: ActionId::Quit,
        label: "Quit",
        keys: "q",
    },
];

/// Options shown in the WhichKey hint while a prefix is pending.
pub fn prefix_options(prefix: char) -> Vec<(&'static str, &'static str)> {
    match prefix {
        'z' => vec![
            ("a", "group: agent"),
            ("r", "group: repo"),
            ("A", "group: agent▸repo"),
            ("d", "group: month"),
            ("f", "group: flat"),
            ("c", "collapse all"),
            ("o", "expand all"),
        ],
        's' => vec![
            ("d", "sort: newest"),
            ("D", "sort: oldest"),
            ("t", "sort: title"),
        ],
        'g' => vec![("g", "go to top")],
        _ => Vec::new(),
    }
}

/// Resolve a completed key progression to an action.
pub const fn resolve_progression(prefix: char, key: char) -> Option<ActionId> {
    match (prefix, key) {
        ('z', 'a') => Some(ActionId::GroupAgent),
        ('z', 'r') => Some(ActionId::GroupRepo),
        ('z', 'A') => Some(ActionId::GroupAgentRepo),
        ('z', 'd') => Some(ActionId::GroupDate),
        ('z', 'f') => Some(ActionId::GroupFlat),
        ('z', 'c') => Some(ActionId::CollapseAll),
        ('z', 'o') => Some(ActionId::ExpandAll),
        ('s', 'd') => Some(ActionId::SortDateDesc),
        ('s', 'D') => Some(ActionId::SortDateAsc),
        ('s', 't') => Some(ActionId::SortTitle),
        _ => None,
    }
}

// =============================================================================
// Command palette
// =============================================================================

pub struct PaletteState {
    pub query: String,
    pub cursor: usize,
    /// Indices into `ACTIONS`, filtered by the query.
    pub matches: Vec<usize>,
}

impl PaletteState {
    pub fn new() -> Self {
        let mut s = Self {
            query: String::new(),
            cursor: 0,
            matches: Vec::new(),
        };
        s.refilter();
        s
    }

    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let mut scored: Vec<(i64, usize)> = ACTIONS
            .iter()
            .enumerate()
            .filter_map(|(i, a)| fuzzy_score(&a.label.to_lowercase(), &q).map(|s| (s, i)))
            .collect();
        scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        self.matches = scored.into_iter().map(|(_, i)| i).collect();
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<ActionId> {
        self.matches.get(self.cursor).map(|&i| ACTIONS[i].id)
    }
}

/// Simple subsequence fuzzy match; higher is better, None = no match.
fn fuzzy_score(haystack: &str, needle: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let mut score: i64 = 0;
    let mut last_pos: Option<usize> = None;
    let hay: Vec<char> = haystack.chars().collect();
    let mut start = 0usize;
    for nc in needle.chars() {
        let mut found = None;
        for (i, hc) in hay.iter().enumerate().skip(start) {
            if *hc == nc {
                found = Some(i);
                break;
            }
        }
        let pos = found?;
        score += match last_pos {
            Some(lp) if pos == lp + 1 => 5,
            _ => 1,
        };
        if pos == 0 {
            score += 3;
        }
        last_pos = Some(pos);
        start = pos + 1;
    }
    Some(score)
}
