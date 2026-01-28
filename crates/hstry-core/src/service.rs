use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::{SearchMode, SearchOptions};
use crate::models::SearchHit;
use crate::paths::service_port_path;

#[expect(clippy::allow_attributes)]
#[expect(clippy::default_trait_access)]
pub mod proto {
    tonic::include_proto!("hstry.service");
}

pub use proto::search_service_client::SearchServiceClient;
pub use proto::search_service_server::{SearchService, SearchServiceServer};

pub fn search_mode_to_proto(mode: SearchMode) -> proto::SearchMode {
    match mode {
        SearchMode::Auto => proto::SearchMode::Auto,
        SearchMode::NaturalLanguage => proto::SearchMode::Natural,
        SearchMode::Code => proto::SearchMode::Code,
    }
}

pub fn search_mode_from_proto(mode: i32) -> SearchMode {
    match proto::SearchMode::try_from(mode) {
        Ok(proto::SearchMode::Natural) => SearchMode::NaturalLanguage,
        Ok(proto::SearchMode::Code) => SearchMode::Code,
        _ => SearchMode::Auto,
    }
}

pub fn search_request_to_opts(request: &proto::SearchRequest) -> SearchOptions {
    SearchOptions {
        source_id: if request.source.is_empty() {
            None
        } else {
            Some(request.source.clone())
        },
        workspace: if request.workspace.is_empty() {
            None
        } else {
            Some(request.workspace.clone())
        },
        limit: if request.limit > 0 {
            Some(request.limit)
        } else {
            None
        },
        offset: if request.offset > 0 {
            Some(request.offset)
        } else {
            None
        },
        mode: search_mode_from_proto(request.mode),
    }
}

fn search_request_from_opts(query: &str, opts: &SearchOptions) -> proto::SearchRequest {
    proto::SearchRequest {
        query: query.to_string(),
        limit: opts.limit.unwrap_or(0),
        offset: opts.offset.unwrap_or(0),
        source: opts.source_id.clone().unwrap_or_default(),
        workspace: opts.workspace.clone().unwrap_or_default(),
        mode: search_mode_to_proto(opts.mode) as i32,
    }
}

pub async fn try_service_search(
    query: &str,
    opts: &SearchOptions,
) -> crate::Result<Option<Vec<SearchHit>>> {
    if std::env::var("HSTRY_NO_SERVICE").is_ok() {
        return Ok(None);
    }

    let port = if let Ok(value) = std::env::var("HSTRY_SERVICE_PORT") {
        value.parse::<u16>().ok()
    } else {
        std::fs::read_to_string(service_port_path())
            .ok()
            .and_then(|value| value.trim().parse::<u16>().ok())
    };

    let Some(port) = port else {
        return Ok(None);
    };

    let endpoint = format!("http://127.0.0.1:{port}");
    let Ok(mut client) = SearchServiceClient::connect(endpoint).await else {
        return Ok(None);
    };

    let request = search_request_from_opts(query, opts);
    let Ok(response) = client.search(request).await else {
        return Ok(None);
    };

    let hits = response
        .into_inner()
        .hits
        .into_iter()
        .map(hit_from_proto)
        .collect();
    Ok(Some(hits))
}

fn ts_ms(dt: Option<DateTime<Utc>>) -> i64 {
    dt.map_or(0, |v| v.timestamp_millis())
}

fn ts_from_ms(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        None
    } else {
        chrono::DateTime::from_timestamp_millis(value).map(|dt| dt.with_timezone(&Utc))
    }
}

pub fn hit_to_proto(hit: &SearchHit) -> proto::SearchHit {
    proto::SearchHit {
        message_id: hit.message_id.to_string(),
        conversation_id: hit.conversation_id.to_string(),
        message_idx: hit.message_idx,
        role: hit.role.to_string(),
        content: hit.content.clone(),
        snippet: hit.snippet.clone(),
        created_at_ms: ts_ms(hit.created_at),
        conv_created_at_ms: hit.conv_created_at.timestamp_millis(),
        conv_updated_at_ms: ts_ms(hit.conv_updated_at),
        score: hit.score,
        source_id: hit.source_id.clone(),
        external_id: hit.external_id.clone().unwrap_or_default(),
        title: hit.title.clone().unwrap_or_default(),
        workspace: hit.workspace.clone().unwrap_or_default(),
        source_adapter: hit.source_adapter.clone(),
        source_path: hit.source_path.clone().unwrap_or_default(),
        host: hit.host.clone().unwrap_or_default(),
    }
}

pub fn hit_from_proto(hit: proto::SearchHit) -> SearchHit {
    SearchHit {
        message_id: Uuid::parse_str(&hit.message_id).unwrap_or_default(),
        conversation_id: Uuid::parse_str(&hit.conversation_id).unwrap_or_default(),
        message_idx: hit.message_idx,
        role: hit.role.as_str().into(),
        content: hit.content,
        snippet: hit.snippet,
        created_at: ts_from_ms(hit.created_at_ms),
        conv_created_at: ts_from_ms(hit.conv_created_at_ms).unwrap_or_else(Utc::now),
        conv_updated_at: ts_from_ms(hit.conv_updated_at_ms),
        score: hit.score,
        source_id: hit.source_id,
        external_id: if hit.external_id.is_empty() {
            None
        } else {
            Some(hit.external_id)
        },
        title: if hit.title.is_empty() {
            None
        } else {
            Some(hit.title)
        },
        workspace: if hit.workspace.is_empty() {
            None
        } else {
            Some(hit.workspace)
        },
        source_adapter: hit.source_adapter,
        source_path: if hit.source_path.is_empty() {
            None
        } else {
            Some(hit.source_path)
        },
        host: if hit.host.is_empty() {
            None
        } else {
            Some(hit.host)
        },
    }
}
