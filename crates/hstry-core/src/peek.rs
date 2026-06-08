//! Token-efficient session previews ("peek bundles").
//!
//! A peek bundle is a small, deterministic, adapter-agnostic gist of a session
//! built from already-stored conversation + message data. It is designed for
//! agents that need to triage many sessions cheaply (typical target: 1-2 KB
//! per session vs ~700 KB for a full `show`).
//!
//! See trx-52b6 for the feature spec.

use crate::models::{Conversation, Message, MessageRole};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Configurable truncation knobs. All defaults match `hstry list --peek`.
#[derive(Debug, Clone)]
pub struct PeekConfig {
    pub first_user_chars: usize,
    pub last_user_chars: usize,
    pub last_assistant_chars: usize,
    pub bash_sample_count: usize,
    pub bash_sample_chars: usize,
    pub files_touched_max: usize,
}

impl Default for PeekConfig {
    fn default() -> Self {
        Self {
            first_user_chars: 240,
            last_user_chars: 240,
            last_assistant_chars: 400,
            bash_sample_count: 6,
            bash_sample_chars: 80,
            files_touched_max: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeekCounts {
    pub user: u32,
    pub assistant: u32,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeekBundle {
    pub id: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub duration_min: i64,
    pub message_count: i64,
    pub counts: PeekCounts,
    pub tools: BTreeMap<String, u32>,
    pub files_touched: Vec<String>,
    pub bash_sample: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_assistant: Option<String>,
}

pub fn build_peek(conv: &Conversation, messages: &[Message], cfg: &PeekConfig) -> PeekBundle {
    let mut counts = PeekCounts {
        user: 0,
        assistant: 0,
        tool_calls: 0,
    };
    let mut tools: BTreeMap<String, u32> = BTreeMap::new();
    let mut files: Vec<String> = Vec::new();
    let mut bash_seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut bash_sample: Vec<String> = Vec::new();

    let mut first_user_idx: Option<usize> = None;
    let mut last_user_idx: Option<usize> = None;
    let mut last_assistant_idx: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            MessageRole::User => {
                if has_text_content(msg) {
                    counts.user += 1;
                    if first_user_idx.is_none() {
                        first_user_idx = Some(i);
                    }
                    last_user_idx = Some(i);
                }
            }
            MessageRole::Assistant => {
                counts.assistant += 1;
                last_assistant_idx = Some(i);
            }
            _ => {}
        }

        if let Some(parts) = msg.parts_json.as_array() {
            for part in parts {
                let Some(t) = part.get("type").and_then(|v| v.as_str()) else {
                    continue;
                };
                if t != "tool_call" {
                    continue;
                }
                counts.tool_calls += 1;

                let name = part
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                if name.is_empty() {
                    continue;
                }
                *tools.entry(name.clone()).or_insert(0) += 1;

                let input = part.get("input");

                if name == "bash" {
                    if let Some(cmd) = input
                        .and_then(|v| v.get("command"))
                        .and_then(|v| v.as_str())
                    {
                        let truncated = truncate_chars(cmd.trim(), cfg.bash_sample_chars);
                        scan_paths_into(cmd, &mut files);
                        if !bash_seen.contains_key(&truncated) {
                            bash_seen.insert(truncated.clone(), ());
                            if bash_sample.len() < cfg.bash_sample_count {
                                bash_sample.push(truncated);
                            }
                        }
                    }
                } else if let Some(obj) = input.and_then(|v| v.as_object()) {
                    for key in ["file_path", "path", "filePath", "notebook_path"] {
                        if let Some(p) = obj.get(key).and_then(|v| v.as_str())
                            && !p.is_empty()
                        {
                            files.push(p.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    dedup_preserve_order(&mut files);
    if files.len() > cfg.files_touched_max {
        files.truncate(cfg.files_touched_max);
    }

    let first_user =
        first_user_idx.map(|i| truncate_chars(&messages[i].content, cfg.first_user_chars));
    let last_user =
        last_user_idx.map(|i| truncate_chars(&messages[i].content, cfg.last_user_chars));
    let last_assistant =
        last_assistant_idx.map(|i| truncate_chars(&messages[i].content, cfg.last_assistant_chars));

    let duration_min = match (conv.created_at, conv.updated_at) {
        (created, Some(updated)) => (updated - created).num_minutes(),
        _ => 0,
    };

    PeekBundle {
        id: conv.id.to_string(),
        source: conv.source_id.clone(),
        model: conv.model.clone(),
        created_at: conv.created_at,
        duration_min,
        message_count: messages.len() as i64,
        counts,
        tools,
        files_touched: files,
        bash_sample,
        first_user,
        last_user,
        last_assistant,
    }
}

fn has_text_content(msg: &Message) -> bool {
    // Adapters that emit parts (claude-code, pi, etc.) often pack tool results
    // into user messages with non-empty content (the tool's stdout). Those
    // should not count as "real" user turns. When parts are present, trust
    // them: a real user turn has at least one text part.
    if let Some(parts) = msg.parts_json.as_array()
        && !parts.is_empty()
    {
        return parts.iter().any(|p| {
            p.get("type")
                .and_then(|v| v.as_str())
                .map(|t| t == "text")
                .unwrap_or(false)
        });
    }
    !msg.content.trim().is_empty()
}

/// Truncate `s` to at most `max_chars` Unicode scalar values (not bytes),
/// breaking on a char boundary. Adds no ellipsis.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    out
}

fn dedup_preserve_order(v: &mut Vec<String>) {
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    v.retain(|s| seen.insert(s.clone(), ()).is_none());
}

/// Extract filesystem path tokens from a string (typically a bash command).
/// Catches absolute (`/...`), home-relative (`~/...`), and dot-relative
/// (`./...`) tokens that contain a `.ext`. Conservative on purpose: we'd
/// rather miss a path than fabricate one. URLs (`http://`, `https://`) are
/// rejected because their leading scheme is followed by `//` not a real path.
fn scan_paths_into(s: &str, out: &mut Vec<String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start_ok = i == 0
            || matches!(
                bytes[i - 1],
                b' ' | b'\t' | b'\n' | b'`' | b'"' | b'\'' | b'=' | b'(' | b';' | b'|' | b'&'
            );
        if !start_ok {
            i += 1;
            continue;
        }
        let is_root = bytes[i] == b'/';
        let is_home = i + 1 < bytes.len() && bytes[i] == b'~' && bytes[i + 1] == b'/';
        let is_dot = i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b'/';
        if !(is_root || is_home || is_dot) {
            i += 1;
            continue;
        }
        if is_root && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // probably a URL scheme tail
            i += 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() {
            let b = bytes[j];
            let allowed =
                b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'~');
            if !allowed {
                break;
            }
            j += 1;
        }
        if j > i {
            let token = &s[i..j];
            // Trim trailing punctuation like `,` `:` `;` that aren't in the
            // allowed set already, plus trailing `/` which usually means
            // "directory mentioned, not a file path".
            let trimmed = token.trim_end_matches('/');
            if is_plausible_path(trimmed) {
                out.push(trimmed.to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

/// Heuristic: a path-shaped token is plausible if it has at least one
/// non-leading `/` (so `/etc/hosts` ✓ but bare `/foo` ✗ without a dot),
/// or contains a `.` extension on its last segment. Rejects bare dotfiles
/// (`./.env` → last segment `.env` starts with `.`).
fn is_plausible_path(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    let trimmed = s.trim_start_matches("~/").trim_start_matches("./");
    let after_root = trimmed.trim_start_matches('/');
    if after_root.is_empty() {
        return false;
    }
    let inner_slashes = after_root.matches('/').count();
    if inner_slashes >= 1 {
        return true;
    }
    if let Some(last) = s.rsplit('/').next()
        && last.contains('.')
        && !last.starts_with('.')
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use uuid::Uuid;

    fn conv() -> Conversation {
        Conversation {
            id: Uuid::nil(),
            source_id: "claude-code".to_string(),
            external_id: Some("ext-1".to_string()),
            readable_id: None,
            platform_id: None,
            title: None,
            created_at: chrono::Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap(),
            updated_at: Some(chrono::Utc.with_ymd_and_hms(2026, 5, 1, 11, 30, 0).unwrap()),
            model: Some("claude-opus-4-7".to_string()),
            provider: None,
            workspace: Some("/repo".to_string()),
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::Value::Null,
            harness: None,
            version: 1,
            message_count: 0,
            parent_conversation_id: None,
            parent_message_idx: None,
            fork_type: None,
        }
    }

    fn msg(idx: i32, role: MessageRole, content: &str, parts: serde_json::Value) -> Message {
        Message {
            id: Uuid::new_v4(),
            conversation_id: Uuid::nil(),
            idx,
            role,
            content: content.to_string(),
            parts_json: parts,
            created_at: None,
            model: None,
            tokens: None,
            cost_usd: None,
            metadata: serde_json::Value::Null,
            sender: None,
            provider: None,
            harness: None,
            client_id: None,
        }
    }

    #[test]
    fn builds_minimal_bundle_for_text_only_session() {
        let c = conv();
        let messages = vec![
            msg(
                0,
                MessageRole::User,
                "fix the bug",
                json!([{"type":"text","text":"fix the bug"}]),
            ),
            msg(
                1,
                MessageRole::Assistant,
                "done",
                json!([{"type":"text","text":"done"}]),
            ),
        ];
        let b = build_peek(&c, &messages, &PeekConfig::default());
        assert_eq!(b.counts.user, 1);
        assert_eq!(b.counts.assistant, 1);
        assert_eq!(b.counts.tool_calls, 0);
        assert_eq!(b.first_user.as_deref(), Some("fix the bug"));
        assert_eq!(b.last_assistant.as_deref(), Some("done"));
        assert!(b.tools.is_empty());
        assert!(b.files_touched.is_empty());
        assert_eq!(b.duration_min, 90);
    }

    #[test]
    fn extracts_files_from_edit_and_bash_tool_calls() {
        let c = conv();
        let messages = vec![
            msg(
                0,
                MessageRole::User,
                "refactor",
                json!([{"type":"text","text":"refactor"}]),
            ),
            msg(
                1,
                MessageRole::Assistant,
                "",
                json!([
                    {"type":"tool_call","name":"Edit","input":{"file_path":"/repo/src/lib.rs"}},
                    {"type":"tool_call","name":"Bash","input":{"command":"cargo test -p hstry-core"}},
                    {"type":"tool_call","name":"Bash","input":{"command":"cat /repo/Cargo.toml | head"}}
                ]),
            ),
        ];
        let b = build_peek(&c, &messages, &PeekConfig::default());
        assert_eq!(b.counts.tool_calls, 3);
        assert_eq!(b.tools.get("edit"), Some(&1));
        assert_eq!(b.tools.get("bash"), Some(&2));
        assert!(b.files_touched.contains(&"/repo/src/lib.rs".to_string()));
        assert!(b.files_touched.contains(&"/repo/Cargo.toml".to_string()));
        assert_eq!(b.bash_sample.len(), 2);
    }

    #[test]
    fn pure_tool_call_assistant_turns_still_counted() {
        let c = conv();
        let messages = vec![
            msg(
                0,
                MessageRole::User,
                "look",
                json!([{"type":"text","text":"look"}]),
            ),
            msg(
                1,
                MessageRole::Assistant,
                "Bash: trx ready",
                json!([{"type":"tool_call","name":"Bash","input":{"command":"trx ready"}}]),
            ),
        ];
        let b = build_peek(&c, &messages, &PeekConfig::default());
        assert_eq!(b.counts.assistant, 1);
        assert_eq!(b.counts.tool_calls, 1);
    }

    #[test]
    fn deduplicates_bash_commands_in_sample() {
        let c = conv();
        let messages = vec![msg(
            0,
            MessageRole::Assistant,
            "",
            json!([
                {"type":"tool_call","name":"Bash","input":{"command":"git status"}},
                {"type":"tool_call","name":"Bash","input":{"command":"git status"}},
                {"type":"tool_call","name":"Bash","input":{"command":"git log -1"}}
            ]),
        )];
        let b = build_peek(&c, &messages, &PeekConfig::default());
        assert_eq!(
            b.bash_sample,
            vec!["git status".to_string(), "git log -1".to_string()]
        );
    }

    #[test]
    fn truncates_long_messages() {
        let c = conv();
        let big = "a".repeat(1000);
        let messages = vec![msg(
            0,
            MessageRole::User,
            &big,
            json!([{"type":"text","text":big}]),
        )];
        let cfg = PeekConfig {
            first_user_chars: 50,
            ..Default::default()
        };
        let b = build_peek(&c, &messages, &cfg);
        assert_eq!(b.first_user.as_ref().unwrap().chars().count(), 50);
    }

    #[test]
    fn tool_result_only_user_turn_does_not_count_as_user() {
        let c = conv();
        // claude-code packs tool_results in synthetic user messages; they
        // should not inflate the user count.
        let messages = vec![
            msg(
                0,
                MessageRole::User,
                "fix",
                json!([{"type":"text","text":"fix"}]),
            ),
            msg(
                1,
                MessageRole::User,
                "",
                json!([{"type":"tool_result","toolCallId":"x","output":"ok"}]),
            ),
        ];
        let b = build_peek(&c, &messages, &PeekConfig::default());
        assert_eq!(b.counts.user, 1);
    }

    #[test]
    fn path_scanner_rejects_urls_and_bare_dotfiles() {
        let mut out = Vec::new();
        scan_paths_into(
            "curl https://example.com/index.html /etc/hosts ./.env",
            &mut out,
        );
        assert!(out.iter().any(|p| p == "/etc/hosts"));
        assert!(!out.iter().any(|p| p.starts_with("https")));
        // ./.env begins with a hidden file — rsplit('/').next() = ".env",
        // which starts with '.', so the has_ext check rejects it.
        assert!(!out.iter().any(|p| p == "./.env"));
    }
}
