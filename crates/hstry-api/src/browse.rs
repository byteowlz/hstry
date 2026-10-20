//! Read endpoints behind the hstry web UI.
//!
//! Every payload is the same `{ok, result, error}` envelope and core type the
//! CLI prints with `--json` (`hstry list`, `hstry show --full`,
//! `hstry source list`), so the UI reads identical data whether it is served
//! here or runs as an Oqto App whose operations call the CLI.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use hstry_core::db::ListConversationsOptions;
use hstry_core::listing::{ListSessionsOptions, list_sessions};
use hstry_core::models::{ConversationWithMessages, MessageWithExtras};

use crate::AppState;

/// The single-file web UI, built from `web/` by `just ui-build`.
const UI_HTML: &str = include_str!("../ui/index.html");

/// Everything the page needs is inlined; it may only call back to this server.
const UI_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
img-src 'self' data: blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn ok(result: impl serde::Serialize) -> Json<Value> {
    Json(json!({ "ok": true, "result": result, "error": null }))
}

fn fail(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "ok": false, "result": null, "error": message.into() })),
    )
}

pub async fn ui() -> Response {
    let mut response = UI_HTML.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(UI_CSP),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Debug, Deserialize)]
pub struct SessionsQuery {
    limit: Option<i64>,
    source: Option<String>,
    workspace: Option<String>,
    /// Include resume/compaction continuation fragments.
    #[serde(default)]
    all: bool,
}

/// `GET /sessions` — same result as `hstry list --json`.
pub async fn sessions(
    State(state): State<AppState>,
    Query(q): Query<SessionsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let previews = list_sessions(
        &state.db,
        ListSessionsOptions {
            list: ListConversationsOptions {
                source_id: q.source,
                workspace: q.workspace.map(|w| format!("%{w}%")),
                limit: Some(q.limit.unwrap_or(5000).clamp(1, 20_000)),
                ..Default::default()
            },
            include_fragments: q.all,
        },
    )
    .await
    .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(ok(previews))
}

/// `GET /sessions/{id}` — same result as `hstry show --full --json <id>`.
pub async fn session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id: Uuid = id
        .parse()
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "session id must be a UUID"))?;
    let conversation = state
        .db
        .get_conversation(id)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("No conversation {id}")))?;
    let messages = state
        .db
        .get_messages(id)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(ok(ConversationWithMessages {
        conversation,
        messages: messages
            .into_iter()
            .map(|message| MessageWithExtras {
                message,
                tool_calls: Vec::new(),
                attachments: Vec::new(),
            })
            .collect(),
    }))
}

/// `GET /sources` — same result as `hstry source list --json`.
pub async fn sources(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let sources = state
        .db
        .list_sources()
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(ok(sources))
}

/// `GET /agents` — resume targets for the "open in" menu (names + default).
pub async fn agents(State(state): State<AppState>) -> Json<Value> {
    let mut names: Vec<&String> = state.config.resume.agents.keys().collect();
    names.sort();
    ok(json!({ "default": state.config.resume.default_agent, "agents": names }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hstry_core::ingest::ingest_batch;
    use hstry_core::models::Source;
    use hstry_core::{Config, Database};

    use super::*;

    async fn state() -> anyhow::Result<(tempfile::TempDir, AppState, Uuid)> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(&dir.path().join("api.db")).await?;
        db.upsert_source(&Source {
            id: "pi".into(),
            adapter: "pi".into(),
            path: Some("/home/me/.pi/agent/sessions".into()),
            last_sync_at: None,
            config: json!({}),
        })
        .await?;
        let conv = serde_json::from_value(json!({
            "externalId": "s1",
            "title": "Fix flaky retry test",
            "workspace": "/work/hstry",
            "createdAt": 1767225600000_i64,
            "messages": [
                {"role": "user", "content": "why does it flake?"},
                {"role": "assistant", "content": "", "parts": [
                    {"id": "p1", "type": "tool_call", "toolCallId": "t1", "name": "bash", "input": {"command": "cargo test"}}
                ]}
            ]
        }))?;
        let fragment = serde_json::from_value(json!({
            "externalId": "s2",
            "createdAt": 1767225700000_i64,
            "messages": [{"role": "user", "content": "The conversation history before this point was compacted into a summary"}]
        }))?;
        ingest_batch(&db, "pi", vec![conv, fragment]).await?;
        let id = db
            .list_conversations(ListConversationsOptions::default())
            .await?
            .into_iter()
            .find(|c| c.external_id.as_deref() == Some("s1"))
            .map(|c| c.id)
            .ok_or_else(|| anyhow::anyhow!("fixture missing"))?;
        let state = AppState {
            config: Arc::new(Config::default()),
            db: Arc::new(db),
            ingest_token: Arc::new(None),
        };
        Ok((dir, state, id))
    }

    #[tokio::test]
    async fn sessions_match_hstry_list_json() -> anyhow::Result<()> {
        let (_dir, state, id) = state().await?;
        let query: SessionsQuery = serde_json::from_value(json!({}))?;
        let Json(body) = sessions(State(state.clone()), Query(query))
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
        assert_eq!(body["ok"], true);
        let list = body["result"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("no list"))?;
        // Continuation fragments are hidden by default, like `hstry list`.
        assert_eq!(list.len(), 1, "{body}");
        assert_eq!(list[0]["id"], id.to_string());
        assert_eq!(list[0]["title"], "Fix flaky retry test");
        assert_eq!(list[0]["first_user_message"], "why does it flake?");

        let query: SessionsQuery = serde_json::from_value(json!({"all": true}))?;
        let Json(all) = sessions(State(state), Query(query))
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
        assert_eq!(all["result"].as_array().map(Vec::len), Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn session_returns_the_full_transcript_with_parts() -> anyhow::Result<()> {
        let (_dir, state, id) = state().await?;
        let Json(body) = session(State(state.clone()), Path(id.to_string()))
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
        assert_eq!(body["result"]["title"], "Fix flaky retry test");
        assert_eq!(
            body["result"]["messages"][1]["parts_json"][0]["name"],
            "bash"
        );

        let missing = session(State(state.clone()), Path(Uuid::nil().to_string())).await;
        assert_eq!(
            missing.map(|_| ()).map_err(|e| e.0),
            Err(StatusCode::NOT_FOUND)
        );
        let invalid = session(State(state), Path("nope".into())).await;
        assert_eq!(
            invalid.map(|_| ()).map_err(|e| e.0),
            Err(StatusCode::BAD_REQUEST)
        );
        Ok(())
    }

    #[tokio::test]
    async fn sources_and_agents_are_listed() -> anyhow::Result<()> {
        let (_dir, state, _) = state().await?;
        let Json(body) = sources(State(state.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
        assert_eq!(body["result"][0]["adapter"], "pi");
        let Json(agents) = agents(State(state)).await;
        assert!(
            agents["result"]["agents"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        );
        assert!(agents["result"]["default"].is_string());
        Ok(())
    }

    #[tokio::test]
    async fn ui_is_served_with_a_locked_down_csp() {
        let response = ui().await;
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }
}
