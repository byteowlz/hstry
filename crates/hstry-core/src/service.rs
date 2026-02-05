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

pub use proto::read_service_client::ReadServiceClient;
pub use proto::read_service_server::{ReadService, ReadServiceServer};
pub use proto::search_service_client::SearchServiceClient;
pub use proto::search_service_server::{SearchService, SearchServiceServer};
pub use proto::write_service_client::WriteServiceClient;
pub use proto::write_service_server::{WriteService, WriteServiceServer};

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

    // Try to connect (Unix socket first, then TCP)
    let Some(mut client) = try_connect_search_client().await else {
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

/// Try to connect to the search service.
/// Attempts Unix socket first (if available), then falls back to TCP.
async fn try_connect_search_client() -> Option<SearchServiceClient<tonic::transport::Channel>> {
    // Try Unix socket first (more secure)
    #[cfg(unix)]
    {
        use crate::paths::service_socket_path;
        let socket_path = service_socket_path();
        if socket_path.exists() {
            if let Some(client) = try_connect_unix(&socket_path).await {
                return Some(client);
            }
        }
    }

    // Fall back to TCP
    let port = if let Ok(value) = std::env::var("HSTRY_SERVICE_PORT") {
        value.parse::<u16>().ok()
    } else {
        read_port_from_paths()
    };

    let port = port?;
    let endpoint = format!("http://127.0.0.1:{port}");
    SearchServiceClient::connect(endpoint).await.ok()
}

#[cfg(unix)]
async fn try_connect_unix(
    socket_path: &std::path::Path,
) -> Option<SearchServiceClient<tonic::transport::Channel>> {
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;
    use tonic::transport::Endpoint;

    let socket_path = socket_path.to_path_buf();

    // Create a channel that connects via Unix socket
    // The URI scheme doesn't matter, we override the connector
    let channel = Endpoint::from_static("http://[::]:0")
        .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
            let path = socket_path.clone();
            async move {
                // Connect to Unix socket and wrap in TokioIo for hyper compatibility
                let stream = UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .ok()?;

    Some(SearchServiceClient::new(channel))
}

fn read_port_from_paths() -> Option<u16> {
    let mut paths = Vec::new();

    let primary = service_port_path();
    paths.push(primary);

    if let Some(home) = dirs::home_dir() {
        let state_fallback = home
            .join(".local")
            .join("state")
            .join("hstry")
            .join("service.port");
        paths.push(state_fallback);

        let config_fallback = home.join(".config").join("hstry").join("service.port");
        paths.push(config_fallback);
    }

    for path in paths {
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(port) = content.trim().parse::<u16>()
        {
            return Some(port);
        }
    }

    None
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

// ============================================================================
// Write Service Conversions
// ============================================================================

use crate::models::{Conversation, Message, MessageEvent, MessageRole};
use crate::parts::Sender;

/// Convert proto Conversation to domain model.
pub fn conversation_from_proto(proto: proto::Conversation) -> Conversation {
    Conversation {
        id: Uuid::new_v4(), // Will be set by upsert based on source_id + external_id
        source_id: proto.source_id,
        external_id: if proto.external_id.is_empty() {
            None
        } else {
            Some(proto.external_id)
        },
        readable_id: None, // Generated by DB
        title: proto.title,
        created_at: ts_from_ms(proto.created_at_ms).unwrap_or_else(Utc::now),
        updated_at: proto.updated_at_ms.and_then(ts_from_ms),
        model: proto.model,
        provider: proto.provider,
        workspace: proto.workspace,
        tokens_in: proto.tokens_in,
        tokens_out: proto.tokens_out,
        cost_usd: proto.cost_usd,
        metadata: if proto.metadata_json.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&proto.metadata_json).unwrap_or_default()
        },
    }
}

/// Convert proto Message to domain model.
pub fn message_from_proto(proto: proto::Message, conversation_id: Uuid) -> Message {
    let sender: Option<Sender> = if proto.sender_json.is_empty() {
        None
    } else {
        serde_json::from_str(&proto.sender_json).ok()
    };

    Message {
        id: Uuid::new_v4(),
        conversation_id,
        idx: proto.idx,
        role: MessageRole::from(proto.role.as_str()),
        content: proto.content,
        parts_json: if proto.parts_json.is_empty() {
            serde_json::json!([])
        } else {
            serde_json::from_str(&proto.parts_json).unwrap_or_else(|_| serde_json::json!([]))
        },
        created_at: proto.created_at_ms.and_then(ts_from_ms),
        model: proto.model,
        tokens: proto.tokens,
        cost_usd: proto.cost_usd,
        metadata: if proto.metadata_json.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&proto.metadata_json).unwrap_or_default()
        },
        sender,
        provider: proto.provider,
    }
}

// ============================================================================
// Read Service Conversions
// ============================================================================

pub fn conversation_to_proto(conv: &Conversation) -> proto::Conversation {
    proto::Conversation {
        source_id: conv.source_id.clone(),
        external_id: conv.external_id.clone().unwrap_or_default(),
        title: conv.title.clone(),
        created_at_ms: conv.created_at.timestamp_millis(),
        updated_at_ms: conv.updated_at.map(|dt| dt.timestamp_millis()),
        model: conv.model.clone(),
        provider: conv.provider.clone(),
        workspace: conv.workspace.clone(),
        tokens_in: conv.tokens_in,
        tokens_out: conv.tokens_out,
        cost_usd: conv.cost_usd,
        metadata_json: conv.metadata.to_string(),
    }
}

pub fn message_to_proto(msg: &Message) -> proto::Message {
    proto::Message {
        idx: msg.idx,
        role: msg.role.to_string(),
        content: msg.content.clone(),
        parts_json: msg.parts_json.to_string(),
        created_at_ms: msg.created_at.map(|dt| dt.timestamp_millis()),
        model: msg.model.clone(),
        tokens: msg.tokens,
        cost_usd: msg.cost_usd,
        metadata_json: msg.metadata.to_string(),
        sender_json: msg
            .sender
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default())
            .unwrap_or_default(),
        provider: msg.provider.clone(),
    }
}

pub fn message_event_to_proto(event: &MessageEvent) -> proto::MessageEvent {
    proto::MessageEvent {
        id: event.id.to_string(),
        conversation_id: event.conversation_id.to_string(),
        idx: event.idx,
        payload_json: event.payload_json.clone(),
        created_at_ms: event
            .created_at
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0),
        metadata_json: event.metadata.to_string(),
    }
}

pub fn conversation_summary_to_proto(
    conv: &Conversation,
    message_count: i64,
    first_user_message: Option<String>,
) -> proto::ConversationSummary {
    proto::ConversationSummary {
        conversation: Some(conversation_to_proto(conv)),
        message_count,
        first_user_message: first_user_message.unwrap_or_default(),
    }
}
