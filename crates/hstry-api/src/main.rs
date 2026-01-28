use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::{Json, Router, extract::Query, extract::State, http::StatusCode, routing::get};
use clap::{Args, Parser};
use log::info;
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use hstry_core::db::{SearchMode, SearchOptions};
use hstry_core::search_tantivy::SearchIndex;
use hstry_core::{Config, Database};

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
    let index_path = config.search_index_path();
    let search_index = SearchIndex::open(&index_path)?;

    let state = AppState {
        config: Arc::new(config),
        db: Arc::new(db),
        search_index: Arc::new(search_index),
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
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], cli.common.port));
    info!("Starting API server on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
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
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    db: Arc<Database>,
    search_index: Arc<SearchIndex>,
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

    let source = params.source.clone();
    let workspace = params.workspace.clone();
    let opts = SearchOptions {
        source_id: source.clone(),
        workspace: workspace.clone(),
        limit: params.limit,
        offset: params.offset,
        mode,
    };

    let results = match state.search_index.search(&params.query, &opts) {
        Ok(results) => results,
        Err(_) => state
            .db
            .search(
                &params.query,
                SearchOptions {
                    source_id: source,
                    workspace,
                    limit: params.limit,
                    offset: params.offset,
                    mode,
                },
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };

    Ok(Json(results))
}
