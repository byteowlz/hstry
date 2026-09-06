//! Shared evidence reads. Budgets include the serialized envelope and its newline.
use crate::{Database, Error, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadOptions {
    pub message_idx: Option<i32>,
    pub message_id: Option<Uuid>,
    pub before: usize,
    pub after: usize,
    pub expand_interactions: bool,
    pub offset: usize,
    pub limit: usize,
    pub max_chars: usize,
    /// `content` or `parts/N/input` / `parts/N/output` (canonical part index).
    pub field: Option<String>,
    pub offset_chars: usize,
    /// Pass the preceding page's version to reject edits during pagination.
    pub version: Option<i64>,
    /// Coordinator-assigned origin label, included in the peer-side wire budget.
    pub machine: Option<String>,
}
impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            message_idx: None,
            message_id: None,
            before: 0,
            after: 0,
            expand_interactions: false,
            offset: 0,
            limit: 50,
            max_chars: 3000,
            field: None,
            offset_chars: 0,
            version: None,
            machine: None,
        }
    }
}
impl ReadOptions {
    pub fn validate(&self) -> Result<()> {
        if !(1000..=1_000_000).contains(&self.max_chars)
            || !(1..=500).contains(&self.limit)
            || self.before > 500
            || self.after > 500
        {
            return Err(Error::Other(
                "max_chars must be 1000..1000000, limit 1..500, before/after 0..500".into(),
            ));
        }
        if self.message_idx.is_some() && self.message_id.is_some() {
            return Err(Error::Other(
                "Choose message_idx or message_id, not both".into(),
            ));
        }
        if self.offset_chars > 0
            && (self.field.is_none() || (self.message_idx.is_none() && self.message_id.is_none()))
        {
            return Err(Error::Other(
                "offset_chars requires an anchored message and explicit field".into(),
            ));
        }
        if self
            .machine
            .as_ref()
            .is_some_and(|m| m.chars().count() > 128)
        {
            return Err(Error::Other("Machine label exceeds 128 characters".into()));
        }
        if let Some(field) = &self.field {
            field_path(field)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRecord {
    pub message_id: Uuid,
    pub message_idx: i32,
    pub role: String,
    pub relation: String,
    pub field: String,
    pub text: String,
    pub offset_chars: usize,
    pub total_chars: usize,
    pub next_offset_chars: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadPage {
    pub protocol: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    pub conversation_id: Uuid,
    pub version: i64,
    pub completeness: String,
    pub absence_is_global: bool,
    pub order: String,
    pub offset: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub truncated: bool,
    pub records: Vec<ReadRecord>,
}
impl ReadPage {
    pub fn to_wire(&self) -> Result<String> {
        Ok(serde_json::to_string(
            &serde_json::json!({"ok":true,"result":self}),
        )?)
    }
}
fn field_path(field: &str) -> Result<Option<String>> {
    if field == "content" {
        return Ok(None);
    }
    let parts: Vec<_> = field.split('/').collect();
    if parts.len() == 3
        && parts[0] == "parts"
        && matches!(parts[2], "input" | "output")
        && let Ok(idx) = parts[1].parse::<u32>()
    {
        return Ok(Some(format!("$[{idx}].{}", parts[2])));
    }
    Err(Error::Other(
        "field must be content or parts/N/input or parts/N/output".into(),
    ))
}
#[derive(Clone)]
struct Anchor {
    id: String,
    idx: i32,
    role: String,
    relation: String,
}

impl Database {
    pub async fn read_page(&self, id: Uuid, opts: ReadOptions) -> Result<ReadPage> {
        opts.validate()?;
        let mut tx = self.read_pool().begin().await?;
        let version: Option<i64> =
            sqlx::query_scalar("SELECT version FROM conversations WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let version = version.ok_or_else(|| Error::Other("Conversation not found".into()))?;
        if opts.version.is_some_and(|v| v != version) {
            return Err(Error::Other(
                "Conversation changed; restart pagination with its new version".into(),
            ));
        }
        let rows =
            sqlx::query("SELECT id,idx,role FROM messages WHERE conversation_id=? ORDER BY idx,id")
                .bind(id.to_string())
                .fetch_all(&mut *tx)
                .await?;
        let all: Vec<Anchor> = rows
            .iter()
            .map(|r| Anchor {
                id: r.get("id"),
                idx: r.get("idx"),
                role: r.get("role"),
                relation: "chronological".into(),
            })
            .collect();
        let anchor = all.iter().position(|m| {
            opts.message_idx == Some(m.idx)
                || opts.message_id.is_some_and(|id| id.to_string() == m.id)
        });
        if anchor.is_none() && (opts.message_idx.is_some() || opts.message_id.is_some()) {
            return Err(Error::Other(
                "Message anchor not found in conversation".into(),
            ));
        }
        let mut selected = if let Some(pos) = anchor {
            let mut v = vec![Anchor {
                relation: "anchor".into(),
                ..all[pos].clone()
            }];
            for (i, m) in all
                .iter()
                .enumerate()
                .take(pos.saturating_add(opts.after).saturating_add(1))
                .skip(pos.saturating_sub(opts.before))
            {
                if i != pos {
                    v.push(Anchor {
                        relation: if i < pos { "before" } else { "after" }.into(),
                        ..m.clone()
                    });
                }
            }
            v
        } else {
            all.clone()
        };
        if opts.expand_interactions {
            expand(&mut tx, id, &all, &mut selected).await?;
        }
        let mut fields = Vec::new();
        for m in selected {
            if let Some(field) = &opts.field {
                fields.push((m, field.clone()));
                continue;
            }
            fields.push((m.clone(), "content".into()));
            let rows=sqlx::query("SELECT p.key AS part, json_extract(p.value,'$.type') AS kind FROM messages m,json_each(m.parts_json) p WHERE m.id=? AND ((json_extract(p.value,'$.type')='tool_call' AND json_type(p.value,'$.input') IS NOT NULL AND json_type(p.value,'$.input')!='null') OR (json_extract(p.value,'$.type')='tool_result' AND json_type(p.value,'$.output') IS NOT NULL AND json_type(p.value,'$.output')!='null'))").bind(&m.id).fetch_all(&mut *tx).await?;
            for r in rows {
                let part: i64 = r.get("part");
                let kind: String = r.get("kind");
                fields.push((
                    m.clone(),
                    format!(
                        "parts/{part}/{}",
                        if kind == "tool_call" {
                            "input"
                        } else {
                            "output"
                        }
                    ),
                ));
            }
        }
        if opts.offset > fields.len() {
            return Err(Error::Other("offset exceeds eligible record count".into()));
        }
        let mut page = ReadPage {
            protocol: 1,
            machine: opts.machine.clone(),
            conversation_id: id,
            version,
            completeness: "unknown".into(),
            absence_is_global: false,
            order: if anchor.is_some() {
                "anchor_first"
            } else {
                "chronological"
            }
            .into(),
            offset: opts.offset,
            total: fields.len(),
            next_offset: None,
            truncated: false,
            records: vec![],
        };
        for (m, field) in fields.iter().skip(opts.offset).take(opts.limit) {
            let text = load_field(&mut tx, &m.id, field).await?;
            let chars: Vec<char> = text.chars().collect();
            let start = if m.relation == "anchor" {
                opts.offset_chars
            } else {
                0
            };
            if start > chars.len() {
                return Err(Error::Other("offset_chars exceeds field length".into()));
            }
            let remaining = chars.len() - start;
            let record = ReadRecord {
                message_id: m
                    .id
                    .parse()
                    .map_err(|_| Error::Other("Invalid stored message ID".into()))?,
                message_idx: m.idx,
                role: m.role.clone(),
                relation: m.relation.clone(),
                field: field.clone(),
                text: String::new(),
                offset_chars: start,
                total_chars: chars.len(),
                next_offset_chars: None,
            };
            page.records.push(record);
            // Reserve continuation/page metadata while finding the largest fitting Unicode prefix.
            let mut low = 0;
            let mut high = remaining.min(opts.max_chars);
            while low < high {
                let mid = low + (high - low).div_ceil(2);
                set_text(&mut page, &chars, start, mid);
                if page.to_wire()?.chars().count() < opts.max_chars {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
            set_text(&mut page, &chars, start, low);
            if page.to_wire()?.chars().count() + 1 > opts.max_chars || (remaining > 0 && low == 0) {
                page.records.pop();
                break;
            }
            if low < remaining {
                break;
            }
        }
        let next = opts.offset + page.records.len();
        page.next_offset = (next < page.total).then_some(next);
        page.truncated = page.next_offset.is_some()
            || page.records.iter().any(|r| r.next_offset_chars.is_some());
        if page.records.is_empty() && page.total > opts.offset {
            return Err(Error::Other(
                "Budget too small for record metadata; increase max_chars".into(),
            ));
        }
        tx.commit().await?;
        Ok(page)
    }
}
fn set_text(page: &mut ReadPage, chars: &[char], start: usize, count: usize) {
    if let Some(r) = page.records.last_mut() {
        r.text = chars[start..start + count].iter().collect();
        r.next_offset_chars = (start + count < chars.len()).then_some(start + count);
    }
    let next = page.offset + page.records.len();
    page.next_offset = (next < page.total).then_some(next);
    page.truncated =
        page.next_offset.is_some() || page.records.iter().any(|r| r.next_offset_chars.is_some());
}
async fn load_field(tx: &mut SqliteConnection, id: &str, field: &str) -> Result<String> {
    if let Some(path) = field_path(field)? {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT CAST(json_extract(parts_json,?) AS TEXT) FROM messages WHERE id=?",
        )
        .bind(path)
        .bind(id)
        .fetch_one(tx)
        .await?;
        return value.ok_or_else(|| Error::Other("Requested tool field is absent".into()));
    }
    Ok(
        sqlx::query_scalar("SELECT content FROM messages WHERE id=?")
            .bind(id)
            .fetch_one(tx)
            .await?,
    )
}
async fn expand(
    tx: &mut SqliteConnection,
    id: Uuid,
    all: &[Anchor],
    selected: &mut Vec<Anchor>,
) -> Result<()> {
    let rows=sqlx::query("SELECT m.id,json_extract(p.value,'$.toolCallId') AS link FROM messages m,json_each(m.parts_json) p WHERE m.conversation_id=? AND json_extract(p.value,'$.type') IN ('tool_call','tool_result') AND json_extract(p.value,'$.toolCallId') IS NOT NULL").bind(id.to_string()).fetch_all(tx).await?;
    let ids: HashSet<_> = selected.iter().map(|m| m.id.as_str()).collect();
    let links: HashSet<String> = rows
        .iter()
        .filter(|r| ids.contains(r.get::<String, _>("id").as_str()))
        .map(|r| r.get("link"))
        .filter(|s: &String| !s.is_empty())
        .collect();
    let extra: HashSet<String> = rows
        .iter()
        .filter(|r| links.contains(&r.get::<String, _>("link")))
        .map(|r| r.get("id"))
        .filter(|s: &String| !ids.contains(s.as_str()))
        .collect();
    if extra.len() > 100 {
        return Err(Error::Other(
            "Interaction expansion exceeds 100 messages; narrow the window".into(),
        ));
    }
    selected.extend(
        all.iter()
            .filter(|m| extra.contains(&m.id))
            .map(|m| Anchor {
                relation: "interaction".into(),
                ..m.clone()
            }),
    );
    Ok(())
}
