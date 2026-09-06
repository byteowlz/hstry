//! Evidence-oriented recall: routing diagnostics, provenance, and wire budgets.
use crate::{
    Database, Result,
    db::{SearchMode, SearchOptions},
    models::{SearchHit, Source},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provenance {
    pub source: String,
    pub machine: Option<String>,
    /// Only an explicit source assertion can establish full/partial; absence means unknown.
    pub completeness: String,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

impl Provenance {
    pub fn apply_snapshot(&mut self, metadata: &Value, local_count: i64) {
        if let Some(expected) = metadata["hstry_sync"]["message_count"].as_i64() {
            self.completeness = if local_count < expected {
                "partial"
            } else {
                "full"
            }
            .into();
            self.basis = Some("copied_snapshot_only".into());
            self.machine = metadata["hstry_sync"]["machine"]
                .as_str()
                .map(str::to_owned)
                .or(self.machine.take());
        }
    }
    pub fn from_source(source: &Source, metadata: &Value) -> Self {
        let completeness = metadata
            .get("completeness")
            .and_then(Value::as_str)
            .or_else(|| source.config.get("completeness").and_then(Value::as_str))
            .filter(|s| matches!(*s, "full" | "partial"))
            .unwrap_or("unknown")
            .to_string();
        Self {
            source: source.id.clone(),
            machine: source
                .config
                .get("machine")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| source.id.split_once(':').map(|(host, _)| host.to_owned())),
            completeness,
            last_sync_at: source.last_sync_at,
            snapshot_at: metadata["hstry_sync"]["captured_at"]
                .as_str()
                .map(str::to_owned),
            basis: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchReport {
    pub hits: Vec<SearchHit>,
    pub attempts: Vec<String>,
    pub stores: Vec<Provenance>,
    pub scope: String,
    pub available_remotes: Vec<String>,
    pub warnings: Vec<String>,
    pub has_more: bool,
    #[serde(default)]
    pub filters: Value,
    #[serde(default)]
    pub offset: i64,
}

pub fn is_needle(query: &str) -> bool {
    let q = query.trim();
    q.contains(['@', '/', '\\', '_', '=', '.', ':', '-'])
        || q.starts_with('"')
        || regex_shaped(q)
        || q.chars()
            .zip(q.chars().skip(1))
            .any(|(a, b)| a.is_lowercase() && b.is_uppercase())
        || (q.len() > 1
            && q.chars().any(char::is_alphabetic)
            && q.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()))
}

pub fn regex_shaped(query: &str) -> bool {
    query.contains(['\\', '[', ']', '(', ')', '|', '*', '+', '?', '^', '$'])
}

pub fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|s| {
            s.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|s| {
            !s.is_empty()
                && ![
                    "what",
                    "which",
                    "where",
                    "when",
                    "how",
                    "did",
                    "do",
                    "does",
                    "we",
                    "i",
                    "a",
                    "an",
                    "the",
                    "for",
                    "about",
                    "in",
                    "on",
                    "of",
                    "to",
                    "is",
                    "was",
                    "it",
                    "set",
                    "session",
                    "sessions",
                    "discuss",
                    "discussed",
                    "decide",
                    "decided",
                    "setup",
                ]
                .contains(&s.as_str())
        })
        .collect()
}

/// Select a dense passage rather than the first occurrence of a topic word.
/// Named field values (JSON/config/assignment output) are stronger evidence than
/// prose merely mentioning the same field. This is query-keyed, never target-keyed.
pub(crate) struct EvidenceQuery {
    words: Vec<String>,
    mentions: Option<regex::Regex>,
    fields: Option<regex::Regex>,
}
impl EvidenceQuery {
    pub fn new(query: &str) -> Self {
        let words = terms(query);
        let alternatives = words
            .iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|");
        let mentions = if alternatives.is_empty() {
            None
        } else {
            regex::Regex::new(&format!("(?i){alternatives}")).ok()
        };
        let fields = if alternatives.is_empty() {
            None
        } else {
            regex::Regex::new(&format!(
                r#"(?i)[a-z0-9_.-]*(?:{alternatives})[a-z0-9_.-]*[\"']?\s*[:=]\s*[\"']?[^\s\"']+"#
            ))
            .ok()
        };
        Self {
            words,
            mentions,
            fields,
        }
    }
    pub fn select(&self, text: &str) -> (Option<usize>, bool) {
        let fields: Vec<_> = self
            .fields
            .as_ref()
            .map(|r| r.find_iter(text).map(|m| m.start()).take(128).collect())
            .unwrap_or_default();
        let mentions: Vec<_> = self
            .mentions
            .as_ref()
            .map(|r| r.find_iter(text).map(|m| m.start()).take(64).collect())
            .unwrap_or_default();
        let best = fields
            .iter()
            .chain(mentions.iter())
            .map(|&pos| {
                let mut start = pos.saturating_sub(75);
                while !text.is_char_boundary(start) {
                    start += 1;
                }
                let excerpt: String = text[start..].chars().take(300).collect();
                let lower = excerpt.to_lowercase();
                let coverage = self
                    .words
                    .iter()
                    .filter(|word| lower.contains(word.as_str()))
                    .count();
                let has_field = fields
                    .iter()
                    .any(|&p| p >= start && p < start + excerpt.len());
                (coverage + usize::from(has_field) * 2, pos, has_field)
            })
            .max_by_key(|(score, pos, _)| (*score, *pos));
        best.map_or((None, false), |(_, p, field)| {
            (Some(text[..p].chars().count()), field)
        })
    }
}

impl Database {
    pub async fn search_report(&self, query: &str, opts: SearchOptions) -> Result<SearchReport> {
        if opts.limit.is_some_and(|n| !(1..=1000).contains(&n))
            || opts.offset.is_some_and(|n| n < 0)
        {
            return Err(crate::Error::Other(
                "limit must be 1..1000; offset must be nonnegative".into(),
            ));
        }
        let mut report = SearchReport {
            scope: "local_snapshot".into(),
            offset: opts.offset.unwrap_or(0),
            filters: json!({"source":opts.source_id,"workspace":opts.workspace,"role":opts.role,"after":opts.after,"before":opts.before,"model":opts.model,"harness":opts.harness,"tag":opts.tag}),
            ..Default::default()
        };
        if let Some(filters) = report.filters.as_object_mut() {
            filters.retain(|_, v| !v.is_null());
        }
        report.stores = self
            .list_sources()
            .await?
            .iter()
            .filter(|s| {
                opts.source_id
                    .as_ref()
                    .is_none_or(|id| s.id == *id || s.id.starts_with(&format!("{id}-")))
            })
            .map(|s| Provenance::from_source(s, &Value::Null))
            .collect();
        let limit = opts.limit.unwrap_or(20);
        let mut pass = opts.clone();
        pass.limit = Some(limit + 1);
        let mut q = query.to_string();
        if query.trim().is_empty() {
            pass.mode = SearchMode::Recent;
        } else if matches!(opts.mode, SearchMode::Needle)
            || opts.mode == SearchMode::Auto && is_needle(query)
        {
            pass.mode = SearchMode::Exact;
            q = query
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(query)
                .to_string();
        } else if opts.mode == SearchMode::Auto {
            pass.mode = SearchMode::NaturalLanguage;
            q = terms(query).join(" ");
        }
        if matches!(pass.mode, SearchMode::NaturalLanguage | SearchMode::Code) {
            pass.offset = None;
            pass.limit = Some(129);
        }
        report.attempts.push(pass.mode.label().into());
        report.hits = self.search_pass(&q, pass.clone()).await?;
        let automatic = matches!(opts.mode, SearchMode::Auto | SearchMode::Needle);
        if report.hits.is_empty()
            && automatic
            && pass.mode == SearchMode::Exact
            && pass.offset.unwrap_or(0) > 0
        {
            let probe = SearchOptions {
                offset: None,
                limit: Some(1),
                ..pass.clone()
            };
            if !self.search_pass(&q, probe).await?.is_empty() {
                report
                    .warnings
                    .push("Exact result page exhausted; earlier pages have matches".into());
                return Ok(report);
            }
        }
        if report.hits.is_empty() && automatic && regex_shaped(query) {
            if regex::Regex::new(query).is_ok() {
                pass.mode = SearchMode::Regex;
                report.attempts.push("regex".into());
                report.hits = self.search_pass(query, pass.clone()).await?;
                if report.hits.is_empty() && pass.offset.unwrap_or(0) > 0 {
                    let probe = SearchOptions {
                        offset: None,
                        limit: Some(1),
                        ..pass.clone()
                    };
                    if !self.search_pass(query, probe).await?.is_empty() {
                        report
                            .warnings
                            .push("Regex result page exhausted; earlier pages have matches".into());
                        return Ok(report);
                    }
                }
            } else {
                report
                    .warnings
                    .push("Invalid regex fallback; continued with FTS".into());
            }
        }
        if report.hits.is_empty() && automatic {
            pass.mode = SearchMode::NaturalLanguage;
            pass.offset = None;
            pass.limit = Some(129);
            if report.attempts.last().is_none_or(|s| s != "natural") {
                report.attempts.push("natural".into());
            }
            q = terms(query).join(" ");
            if !q.is_empty() {
                report.hits = self.search_pass(&q, pass.clone()).await?;
            }
        }
        if automatic && pass.mode == SearchMode::NaturalLanguage && !q.is_empty() {
            report.hits.sort_by(|a, b| a.score.total_cmp(&b.score));
            let mut contexts = Vec::new();
            for hit in &report.hits {
                if !contexts.contains(&hit.conversation_id) {
                    contexts.push(hit.conversation_id);
                }
                if contexts.len() == 5 {
                    break;
                }
            }
            pass.mode = SearchMode::Broad;
            // Search evidence across messages inside the most relevant conversations.
            // This finds a field value even when the topic is mentioned only elsewhere.
            pass.limit = Some(129);
            pass.offset = None;
            report.attempts.push(
                if contexts.is_empty() {
                    "natural_or"
                } else {
                    "conversation_context"
                }
                .into(),
            );
            let expanded = self
                .search_in_conversations(&q, pass.clone(), &contexts)
                .await?;
            let mut seen: std::collections::HashSet<_> =
                report.hits.iter().map(|h| h.message_id).collect();
            report
                .hits
                .extend(expanded.into_iter().filter(|h| seen.insert(h.message_id)));
            if report.hits.len() > 128 {
                report.warnings.push("Broad candidate window capped at 128 messages; narrow the query or search exact for exhaustive literal recall".into());
            }
            let words = terms(&q);
            let mut coverage =
                std::collections::HashMap::<uuid::Uuid, std::collections::HashSet<usize>>::new();
            for hit in &report.hits {
                let text = format!("{} {}", hit.title.as_deref().unwrap_or(""), hit.content)
                    .to_lowercase();
                for (i, word) in words.iter().enumerate() {
                    if text.contains(word) {
                        coverage.entry(hit.conversation_id).or_default().insert(i);
                    }
                }
            }
            report.hits.sort_by(|a, b| {
                coverage
                    .get(&b.conversation_id)
                    .map_or(0, |c| c.len())
                    .cmp(&coverage.get(&a.conversation_id).map_or(0, |c| c.len()))
                    .then_with(|| a.score.total_cmp(&b.score))
                    .then_with(|| a.conversation_id.cmp(&b.conversation_id))
                    .then_with(|| a.message_idx.cmp(&b.message_idx))
            });
        }
        if matches!(pass.mode, SearchMode::NaturalLanguage | SearchMode::Code) {
            report.hits.sort_by(|a, b| {
                a.score
                    .total_cmp(&b.score)
                    .then_with(|| a.source_id.cmp(&b.source_id))
                    .then_with(|| a.external_id.cmp(&b.external_id))
                    .then_with(|| a.conversation_id.cmp(&b.conversation_id))
                    .then_with(|| a.message_idx.cmp(&b.message_idx))
            });
        }
        if matches!(
            pass.mode,
            SearchMode::NaturalLanguage | SearchMode::Code | SearchMode::Broad
        ) {
            if report.hits.len() >= 128 && report.warnings.is_empty() {
                report.warnings.push("Ranked candidate window capped at 128; narrow query or use exact search to continue discovery".into());
            }
            report.hits.truncate(128);
            report.hits = report
                .hits
                .into_iter()
                .skip(opts.offset.unwrap_or(0) as usize)
                .collect();
        }
        report.has_more = report.hits.len() > limit as usize;
        report.hits.truncate(limit as usize);
        Ok(report)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub total: usize,
    pub snippet: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            total: 3000,
            snippet: 300,
        }
    }
}
impl Budget {
    pub fn validate(self) -> Result<Self> {
        if self.total < 512 || self.total > 1_000_000 || self.snippet == 0 || self.snippet > 10_000
        {
            return Err(crate::Error::Other(
                "budget must be 512..1000000 chars; snippet must be 1..10000 chars".into(),
            ));
        }
        Ok(self)
    }
}

pub fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Character-indexed window, never byte-slicing UTF-8. The anchor points into raw content.
pub fn window(text: &str, position: usize, max: usize) -> String {
    text.chars()
        .skip(position.saturating_sub(max / 4))
        .take(max)
        .collect()
}

/// Budget includes the entire compact JSON envelope (keys, escapes, and provenance).
/// Raw mode is explicitly lossless and bypasses presentation budgets.
pub fn project(report: &SearchReport, budget: Budget, raw: bool) -> Result<Value> {
    let budget = budget.validate()?;
    if raw {
        return Ok(json!({"ok":true,"result":report,"error":null}));
    }
    let hits: Vec<Value> = report.hits.iter().map(|h| {
        let snippet = if let Some(pos) = h.match_position { window(&h.content, pos, budget.snippet.min(300)) } else { clip(&h.snippet, budget.snippet.min(300)) };
        json!({"conversation_id":h.conversation_id,"readable_id":h.readable_id.as_deref().map(|s|clip(s,120)),"message_idx":h.message_idx,"role":h.role,"title":h.title.as_deref().map(|s|clip(s,120)),"timestamp":h.created_at.unwrap_or(h.conv_created_at),"snippet":snippet,"match_position":h.match_position,"provenance":h.provenance,"snippet_truncated":snippet.chars().count()<h.content.chars().count()})
    }).collect();
    let mut value = json!({"ok":true,"result":{"hits":hits,"attempts":report.attempts,"scope":report.scope,"stores":report.stores,"available_remotes":report.available_remotes,"warnings":report.warnings,"absence_is_global":false,"truncated":report.has_more,"omitted_hits":0,"has_more":report.has_more,"provenance_truncated":false,"filters":report.filters,"offset":report.offset},"error":null});
    loop {
        let count = value["result"]["hits"].as_array().map_or(0, Vec::len);
        value["result"]["next_offset"] = if value["result"]["truncated"] == true
            && count > 0
            && (report.scope == "local_snapshot" || report.scope.starts_with("remote:"))
        {
            json!(report.offset + count as i64)
        } else {
            Value::Null
        };
        if serde_json::to_string(&value)?.chars().count() < budget.total {
            break;
        }
        let r = &mut value["result"];
        r["truncated"] = json!(true);
        if let Some(stores) = r["stores"].as_array_mut().filter(|a| a.len() > 1) {
            stores.pop();
            r["provenance_truncated"] = json!(true);
        } else if let Some(remotes) = r["available_remotes"]
            .as_array_mut()
            .filter(|a| !a.is_empty())
        {
            remotes.pop();
            r["provenance_truncated"] = json!(true);
        } else if let Some(hits) = r["hits"].as_array_mut().filter(|a| !a.is_empty()) {
            hits.pop();
            r["omitted_hits"] = json!(r["omitted_hits"].as_u64().unwrap_or(0) + 1);
        } else if let Some(stores) = r["stores"].as_array_mut().filter(|a| !a.is_empty()) {
            stores.pop();
            r["provenance_truncated"] = json!(true);
        } else if let Some(warnings) = r["warnings"].as_array_mut().filter(|a| !a.is_empty()) {
            warnings.pop();
        } else if r["filters"].as_object().is_some_and(|o| !o.is_empty()) {
            r["filters"] = json!({});
            r["filters_truncated"] = json!(true);
        } else {
            return Err(crate::Error::Other(
                "budget too small for recall metadata".into(),
            ));
        }
    }
    Ok(value)
}
