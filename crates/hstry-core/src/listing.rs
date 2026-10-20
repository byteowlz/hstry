//! The session list every surface shows: `hstry list`, the HTTP API and UI.
//!
//! One conversation can be imported through several sources (a harness and
//! an `import-<harness>` export of the same session); the list shows each
//! session once, preferring the native source and then the newest copy.
//! Resume/compaction continuation fragments are hidden unless asked for —
//! they stay fully searchable.

use std::collections::HashMap;

use crate::Result;
use crate::db::{ConversationPreview, Database, ListConversationsOptions};
use crate::models::Conversation;

/// Options for [`list_sessions`].
#[derive(Debug, Default)]
pub struct ListSessionsOptions {
    pub list: ListConversationsOptions,
    /// Include resume/compaction continuation fragments.
    pub include_fragments: bool,
}

/// List sessions newest first, deduplicated across sources unless a single
/// source is requested.
pub async fn list_sessions(
    db: &Database,
    opts: ListSessionsOptions,
) -> Result<Vec<ConversationPreview>> {
    let mut list = opts.list;
    let limit = list.limit.unwrap_or(0);
    let dedup = list.source_id.is_none();
    if dedup && limit > 0 {
        // Duplicates collapse after fetching; over-fetch so `limit` still fills.
        list.limit = Some(limit.saturating_mul(4).min(2000));
    }

    let mut fetched = db.list_conversation_previews(list).await?;
    if !opts.include_fragments {
        fetched.retain(|p| !is_continuation_fragment(p.first_user_message.as_deref()));
    }
    if !dedup {
        return Ok(fetched);
    }
    let mut deduped = dedup_conversation_previews(fetched);
    if limit > 0 {
        deduped.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    Ok(deduped)
}

/// A resume/compaction continuation fragment: its first user message is the
/// synthetic "conversation history ... compacted" summary Claude Code injects.
pub fn is_continuation_fragment(first_user: Option<&str>) -> bool {
    first_user.is_some_and(|content| {
        content
            .trim_start()
            .starts_with("The conversation history before this point was compacted")
    })
}

/// Keep one preview per logical session, newest first.
pub fn dedup_conversation_previews(previews: Vec<ConversationPreview>) -> Vec<ConversationPreview> {
    let mut best_by_key: HashMap<String, ConversationPreview> = HashMap::new();
    for preview in previews {
        let key = conversation_identity_key(&preview.conversation);
        match best_by_key.get(&key) {
            Some(existing) if !should_replace(&preview.conversation, &existing.conversation) => {}
            _ => {
                best_by_key.insert(key, preview);
            }
        }
    }
    let mut deduped: Vec<ConversationPreview> = best_by_key.into_values().collect();
    deduped.sort_by_key(|p| std::cmp::Reverse(sort_ts(&p.conversation)));
    deduped
}

fn conversation_identity_key(conv: &Conversation) -> String {
    let harness = conv.harness.as_deref().unwrap_or_default();
    if let Some(external_id) = conv.external_id.as_deref().filter(|v| !v.is_empty()) {
        return format!("{harness}|external|{external_id}");
    }
    if let Some(readable_id) = conv.readable_id.as_deref().filter(|v| !v.is_empty()) {
        return format!("{harness}|readable|{readable_id}");
    }
    if let Some(platform_id) = conv.platform_id.as_deref().filter(|v| !v.is_empty()) {
        return format!("{harness}|platform|{platform_id}");
    }
    format!("{harness}|id|{}", conv.id)
}

fn sort_ts(conv: &Conversation) -> i64 {
    conv.updated_at.unwrap_or(conv.created_at).timestamp()
}

/// Native harness source beats a generic source beats an `import-*` copy.
fn source_preference(source_id: &str, harness: Option<&str>) -> u8 {
    if let Some(harness) = harness {
        if source_id == harness {
            return 0;
        }
        if source_id == format!("import-{harness}") {
            return 2;
        }
    }
    if source_id.starts_with("import-") {
        2
    } else {
        1
    }
}

fn should_replace(candidate: &Conversation, current: &Conversation) -> bool {
    let candidate_pref = source_preference(&candidate.source_id, candidate.harness.as_deref());
    let current_pref = source_preference(&current.source_id, current.harness.as_deref());
    if candidate_pref != current_pref {
        return candidate_pref < current_pref;
    }
    sort_ts(candidate) > sort_ts(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_continuation_fragments() {
        assert!(is_continuation_fragment(Some(
            "The conversation history before this point was compacted into a summary"
        )));
        assert!(!is_continuation_fragment(Some("Central Server for ROMs")));
        assert!(!is_continuation_fragment(None));
    }

    #[test]
    fn native_source_beats_import_copy() {
        assert!(source_preference("pi", Some("pi")) < source_preference("import-pi", Some("pi")));
        assert!(source_preference("other", None) < source_preference("import-x", None));
    }
}
