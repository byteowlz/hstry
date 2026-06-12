use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use clap::{Args, Parser};
use log::info;
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use hstry_core::db::{SearchMode, SearchOptions};
use hstry_core::ingest::ingest_batch;
use hstry_core::models::Source;
use hstry_core::parsed::ParsedConversation;
use hstry_core::{Config, Database};

/// Ingest payloads carry full conversation histories; allow generous bodies.
const INGEST_BODY_LIMIT: usize = 64 * 1024 * 1024;

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn try_main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let config_path = cli
        .common
        .config
        .unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    let db = Database::open(&config.database).await?;

    let ingest_token = cli
        .common
        .token
        .clone()
        .or_else(|| std::env::var("HSTRY_API_TOKEN").ok())
        .filter(|t| !t.is_empty());
    let has_token = ingest_token.is_some();
    if !has_token {
        info!(
            "No ingest token configured (set --token or HSTRY_API_TOKEN); /ingest accepts any loopback client"
        );
    }

    let state = AppState {
        config: Arc::new(config),
        db: Arc::new(db),
        ingest_token: Arc::new(ingest_token),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/config", get(get_config))
        .route("/search", get(search))
        .route(
            "/ingest",
            post(ingest).layer(DefaultBodyLimit::max(INGEST_BODY_LIMIT)),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], cli.common.port));
    info!("Starting API server on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Unconditional banner: env_logger is silent without RUST_LOG, which makes
    // a healthy server look hung. Print one line so the user sees it is up.
    let _ = writeln!(
        io::stderr(),
        "hstry-api listening on http://{addr}  (ingest auth: {}, set RUST_LOG=info,tower_http=debug for request logs)",
        if has_token { "token required" } else { "open on loopback" }
    );
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Debug, Parser)]
#[command(author, version, about = "HTTP API server for rust-workspace")]
struct Cli {
    #[command(flatten)]
    common: CommonOpts,
}

#[derive(Debug, Clone, Args)]
struct CommonOpts {
    /// Override the config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Bearer token required for /ingest (falls back to HSTRY_API_TOKEN)
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    db: Arc<Database>,
    ingest_token: Arc<Option<String>>,
}

#[derive(Serialize)]
struct RootResponse {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn root() -> Json<RootResponse> {
    Json(RootResponse {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn get_config(State(state): State<AppState>) -> Result<Json<Config>, StatusCode> {
    Ok(Json((*state.config).clone()))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    query: String,
    limit: Option<i64>,
    offset: Option<i64>,
    source: Option<String>,
    workspace: Option<String>,
    mode: Option<String>,
    /// ISO 8601 timestamp: only messages after this time
    after: Option<String>,
    /// ISO 8601 timestamp: only messages before this time
    before: Option<String>,
    /// Filter by message role
    role: Option<String>,
    /// Filter by conversation model
    model: Option<String>,
    /// Filter by agent harness
    harness: Option<String>,
    /// Filter by conversation tag
    tag: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<hstry_core::models::SearchHit>>, StatusCode> {
    let mode = match params.mode.as_deref() {
        Some("auto") | None => SearchMode::Auto,
        Some("natural" | "natural_language") => SearchMode::NaturalLanguage,
        Some("code") => SearchMode::Code,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let after = params
        .after
        .as_deref()
        .and_then(|s| dateparser::parse(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let before = params
        .before
        .as_deref()
        .and_then(|s| dateparser::parse(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let source = params.source.clone();
    let workspace = params.workspace.clone();
    let role = params.role.clone();
    let model = params.model.clone();
    let harness = params.harness.clone();
    let tag = params.tag.clone();

    let results = state
        .db
        .search(
            &params.query,
            SearchOptions {
                source_id: source,
                workspace,
                limit: params.limit,
                offset: params.offset,
                mode,
                after,
                before,
                role,
                model,
                harness,
                tag,
            },
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(results))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestRequest {
    /// Source id the conversations belong to (created on first use).
    source: String,
    /// Adapter/provider label stored on a newly created source.
    adapter: Option<String>,
    conversations: Vec<ParsedConversation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestResponse {
    source: String,
    conversations: usize,
    messages: usize,
}

async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, StatusCode> {
    if let Some(expected) = state.ingest_token.as_ref() {
        let provided = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(expected.as_str()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let source_id = req.source.trim();
    if source_id.is_empty()
        || !source_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut source = state
        .db
        .get_source(source_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or_else(|| Source {
            id: source_id.to_string(),
            adapter: req.adapter.clone().unwrap_or_else(|| source_id.to_string()),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        });

    let outcome = ingest_batch(&state.db, source_id, req.conversations)
        .await
        .map_err(|err| {
            log::error!("ingest failed for source '{source_id}': {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !outcome.affected_conversation_ids.is_empty() {
        state
            .db
            .rebuild_conversation_summaries(&outcome.affected_conversation_ids)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    source.last_sync_at = Some(Utc::now());
    state
        .db
        .upsert_source(&source)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(IngestResponse {
        source: source_id.to_string(),
        conversations: outcome.conversations,
        messages: outcome.messages,
    }))
}
