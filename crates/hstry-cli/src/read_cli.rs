use clap::Args;
use hstry_core::read::ReadOptions;

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Message ordinal from search (context is anchor-first).
    #[arg(long, conflicts_with = "message_id")]
    pub message_idx: Option<i32>,
    /// Stable stored message UUID, scoped to this conversation.
    #[arg(long)]
    pub message_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = 0)]
    pub before: usize,
    #[arg(long, default_value_t = 0)]
    pub after: usize,
    /// Include directly linked tool calls/results, never recursive ancestry.
    #[arg(long)]
    pub expand_interactions: bool,
    /// Record-page offset. Finish truncated fields separately with --offset-chars.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Entire serialized response budget, including metadata (1000..1000000).
    #[arg(long, default_value_t = 3000)]
    pub max_chars: usize,
    /// content or parts/N/input or parts/N/output, as returned by a read page.
    #[arg(long)]
    pub field: Option<String>,
    /// Unicode character offset in one anchored field.
    #[arg(long, default_value_t = 0)]
    pub offset_chars: usize,
    /// Reject pagination if the conversation has changed since the last page.
    #[arg(long)]
    pub conversation_version: Option<i64>,
}
impl From<ReadArgs> for ReadOptions {
    fn from(a: ReadArgs) -> Self {
        Self {
            message_idx: a.message_idx,
            message_id: a.message_id,
            before: a.before,
            after: a.after,
            expand_interactions: a.expand_interactions,
            offset: a.offset,
            limit: a.limit,
            max_chars: a.max_chars,
            field: a.field,
            offset_chars: a.offset_chars,
            version: a.conversation_version,
            machine: None,
        }
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    pub id: String,
    pub options: ReadOptions,
}
