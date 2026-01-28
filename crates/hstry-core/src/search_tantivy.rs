//! Tantivy-backed search index for hstry.

use std::path::Path;

use chrono::Utc;
use sqlx::Row;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Query, QueryParser, TermQuery};
use tantivy::schema::{INDEXED, STORED, STRING, Schema, TEXT, TantivyDocument, TextOptions, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use uuid::Uuid;

use crate::Database;
use crate::db::SearchOptions;
use crate::error::{Error, Result};
use crate::models::{MessageRole, SearchHit};

const STATE_KEY: &str = "tantivy_last_rowid";
const WRITER_HEAP_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Fields {
    content: tantivy::schema::Field,
    title: tantivy::schema::Field,
    message_id: tantivy::schema::Field,
    conversation_id: tantivy::schema::Field,
    message_idx: tantivy::schema::Field,
    role: tantivy::schema::Field,
    created_at: tantivy::schema::Field,
    conv_created_at: tantivy::schema::Field,
    conv_updated_at: tantivy::schema::Field,
    source_id: tantivy::schema::Field,
    external_id: tantivy::schema::Field,
    workspace: tantivy::schema::Field,
    source_adapter: tantivy::schema::Field,
    source_path: tantivy::schema::Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();

    let content_opts = TextOptions::default().set_stored().set_indexing_options(
        tantivy::schema::TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );

    let content = builder.add_text_field("content", content_opts);
    let title = builder.add_text_field("title", TEXT | STORED);
    let message_id = builder.add_text_field("message_id", STRING | STORED);
    let conversation_id = builder.add_text_field("conversation_id", STRING | STORED);
    let message_idx = builder.add_i64_field("message_idx", INDEXED | STORED);
    let role = builder.add_text_field("role", STRING | STORED);
    let created_at = builder.add_i64_field("created_at", INDEXED | STORED);
    let conv_created_at = builder.add_i64_field("conv_created_at", INDEXED | STORED);
    let conv_updated_at = builder.add_i64_field("conv_updated_at", INDEXED | STORED);
    let source_id = builder.add_text_field("source_id", STRING | STORED);
    let external_id = builder.add_text_field("external_id", STRING | STORED);
    let workspace = builder.add_text_field("workspace", STRING | STORED);
    let source_adapter = builder.add_text_field("source_adapter", STRING | STORED);
    let source_path = builder.add_text_field("source_path", STRING | STORED);

    let schema = builder.build();
    let fields = Fields {
        content,
        title,
        message_id,
        conversation_id,
        message_idx,
        role,
        created_at,
        conv_created_at,
        conv_updated_at,
        source_id,
        external_id,
        workspace,
        source_adapter,
        source_path,
    };
    (schema, fields)
}

fn resolve_fields(schema: &Schema) -> Result<Fields> {
    let field = |name: &str| {
        schema
            .get_field(name)
            .map_err(|err| Error::Other(format!("Missing Tantivy field: {name} ({err})")))
    };

    Ok(Fields {
        content: field("content")?,
        title: field("title")?,
        message_id: field("message_id")?,
        conversation_id: field("conversation_id")?,
        message_idx: field("message_idx")?,
        role: field("role")?,
        created_at: field("created_at")?,
        conv_created_at: field("conv_created_at")?,
        conv_updated_at: field("conv_updated_at")?,
        source_id: field("source_id")?,
        external_id: field("external_id")?,
        workspace: field("workspace")?,
        source_adapter: field("source_adapter")?,
        source_path: field("source_path")?,
    })
}

fn open_or_create_index(path: &Path) -> Result<(Index, Fields)> {
    if path.exists() {
        let index = Index::open_in_dir(path)
            .map_err(|err| Error::Other(format!("Opening Tantivy index: {err}")))?;
        let fields = resolve_fields(&index.schema())?;
        Ok((index, fields))
    } else {
        std::fs::create_dir_all(path)?;
        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(path, schema)
            .map_err(|err| Error::Other(format!("Creating Tantivy index: {err}")))?;
        Ok((index, fields))
    }
}

fn index_reader(index: &Index) -> Result<IndexReader> {
    index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(|err| Error::Other(format!("Creating Tantivy reader: {err}")))
}

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl SearchIndex {
    pub fn open(path: &Path) -> Result<Self> {
        let (index, fields) = open_or_create_index(path)?;
        let reader = index_reader(&index)?;
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<SearchHit>> {
        search_with_components(&self.index, &self.reader, &self.fields, query, opts)
    }
}

async fn fetch_last_rowid(db: &Database) -> Result<i64> {
    Ok(db
        .get_search_state(STATE_KEY)
        .await?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0))
}

async fn set_last_rowid(db: &Database, value: i64) -> Result<()> {
    db.set_search_state(STATE_KEY, &value.to_string()).await
}

pub async fn reset_index(db: &Database, index_path: &Path) -> Result<()> {
    if index_path.exists() {
        std::fs::remove_dir_all(index_path)?;
    }
    set_last_rowid(db, 0).await
}

fn add_text_field(doc: &mut TantivyDocument, field: tantivy::schema::Field, value: &str) {
    if !value.is_empty() {
        doc.add_text(field, value);
    }
}

fn add_i64_field(doc: &mut TantivyDocument, field: tantivy::schema::Field, value: Option<i64>) {
    if let Some(value) = value {
        doc.add_i64(field, value);
    }
}

async fn load_message_rows(
    db: &Database,
    last_rowid: i64,
    batch_size: usize,
) -> Result<Vec<sqlx::sqlite::SqliteRow>> {
    let rows = sqlx::query(
        r"
        SELECT
            m.rowid AS rowid,
            m.id AS message_id,
            m.conversation_id AS conversation_id,
            m.idx AS message_idx,
            m.role AS role,
            m.content AS content,
            m.created_at AS created_at,
            c.created_at AS conv_created_at,
            c.updated_at AS conv_updated_at,
            c.source_id AS source_id,
            c.external_id AS external_id,
            c.title AS title,
            c.workspace AS workspace,
            s.adapter AS source_adapter,
            s.path AS source_path
        FROM messages m
        JOIN conversations c ON c.id = m.conversation_id
        JOIN sources s ON s.id = c.source_id
        WHERE m.rowid > ?
        ORDER BY m.rowid
        LIMIT ?
        ",
    )
    .bind(last_rowid)
    .bind(i64::try_from(batch_size).unwrap_or(i64::MAX))
    .fetch_all(db.pool())
    .await?;
    Ok(rows)
}

pub async fn index_new_messages(
    db: &Database,
    index_path: &Path,
    batch_size: usize,
) -> Result<usize> {
    let (index, fields) = open_or_create_index(index_path)?;
    let mut writer: IndexWriter = index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|err| Error::Other(format!("Creating Tantivy writer: {err}")))?;

    let mut last_rowid = fetch_last_rowid(db).await?;
    let rows = load_message_rows(db, last_rowid, batch_size).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut indexed = 0usize;
    for row in rows {
        let rowid: i64 = row.get("rowid");
        last_rowid = rowid;

        let mut doc = TantivyDocument::default();
        add_text_field(
            &mut doc,
            fields.message_id,
            row.get::<String, _>("message_id").as_str(),
        );
        add_text_field(
            &mut doc,
            fields.conversation_id,
            row.get::<String, _>("conversation_id").as_str(),
        );
        doc.add_i64(fields.message_idx, row.get::<i64, _>("message_idx"));
        add_text_field(&mut doc, fields.role, row.get::<String, _>("role").as_str());
        add_text_field(
            &mut doc,
            fields.content,
            row.get::<String, _>("content").as_str(),
        );

        add_i64_field(
            &mut doc,
            fields.created_at,
            row.get::<Option<i64>, _>("created_at"),
        );
        doc.add_i64(fields.conv_created_at, row.get::<i64, _>("conv_created_at"));
        add_i64_field(
            &mut doc,
            fields.conv_updated_at,
            row.get::<Option<i64>, _>("conv_updated_at"),
        );

        add_text_field(
            &mut doc,
            fields.source_id,
            row.get::<String, _>("source_id").as_str(),
        );
        if let Some(external_id) = row.get::<Option<String>, _>("external_id") {
            add_text_field(&mut doc, fields.external_id, external_id.as_str());
        }
        if let Some(title) = row.get::<Option<String>, _>("title") {
            add_text_field(&mut doc, fields.title, title.as_str());
        }
        if let Some(workspace) = row.get::<Option<String>, _>("workspace") {
            add_text_field(&mut doc, fields.workspace, workspace.as_str());
        }
        add_text_field(
            &mut doc,
            fields.source_adapter,
            row.get::<String, _>("source_adapter").as_str(),
        );
        if let Some(source_path) = row.get::<Option<String>, _>("source_path") {
            add_text_field(&mut doc, fields.source_path, source_path.as_str());
        }

        writer
            .add_document(doc)
            .map_err(|err| Error::Other(format!("Adding Tantivy document: {err}")))?;
        indexed += 1;
    }

    writer
        .commit()
        .map_err(|err| Error::Other(format!("Committing Tantivy index: {err}")))?;

    set_last_rowid(db, last_rowid).await?;
    Ok(indexed)
}

fn extract_text(doc: &TantivyDocument, field: tantivy::schema::Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|value| value.as_str().map(ToString::to_string))
}

fn extract_i64(doc: &TantivyDocument, field: tantivy::schema::Field) -> Option<i64> {
    doc.get_first(field).and_then(|value| value.as_i64())
}

pub async fn search_with_fallback(
    db: &Database,
    index_path: &Path,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    if !index_path.exists() {
        return db.search(query, opts.clone()).await;
    }
    search(index_path, query, opts)
}

pub fn search(index_path: &Path, query: &str, opts: &SearchOptions) -> Result<Vec<SearchHit>> {
    let index = SearchIndex::open(index_path)?;
    index.search(query, opts)
}

fn search_with_components(
    index: &Index,
    reader: &IndexReader,
    fields: &Fields,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let searcher = reader.searcher();

    let mut parser = QueryParser::for_index(index, vec![fields.content, fields.title]);
    parser.set_conjunction_by_default();
    let parsed_query = parser
        .parse_query(query)
        .map_err(|err| Error::Other(format!("Parsing Tantivy query: {err}")))?;

    let mut filters: Vec<Box<dyn Query>> = vec![parsed_query];

    if let Some(source_id) = &opts.source_id {
        let term = Term::from_field_text(fields.source_id, source_id);
        filters.push(Box::new(TermQuery::new(
            term,
            tantivy::schema::IndexRecordOption::Basic,
        )));
    }

    if let Some(workspace) = &opts.workspace {
        let term = Term::from_field_text(fields.workspace, workspace);
        filters.push(Box::new(TermQuery::new(
            term,
            tantivy::schema::IndexRecordOption::Basic,
        )));
    }

    let query = if filters.len() == 1 {
        filters.remove(0)
    } else {
        Box::new(BooleanQuery::intersection(filters))
    };

    let limit = usize::try_from(opts.limit.unwrap_or(10).max(1)).unwrap_or(1);
    let offset = usize::try_from(opts.offset.unwrap_or(0).max(0)).unwrap_or(0);
    let top_docs = searcher
        .search(&query, &TopDocs::with_limit(limit + offset))
        .map_err(|err| Error::Other(format!("Tantivy search failed: {err}")))?;

    let snippet_gen =
        tantivy::snippet::SnippetGenerator::create(&searcher, &query, fields.content).ok();

    let mut hits = Vec::new();
    for (score, addr) in top_docs.into_iter().skip(offset).take(limit) {
        let doc = searcher
            .doc(addr)
            .map_err(|err| Error::Other(format!("Fetching Tantivy doc: {err}")))?;
        let content = extract_text(&doc, fields.content).unwrap_or_default();
        let snippet = snippet_gen.as_ref().map_or_else(
            || content.clone(),
            |generator| generator.snippet(&content).to_html(),
        );

        let message_id = extract_text(&doc, fields.message_id)
            .and_then(|id| Uuid::parse_str(&id).ok())
            .unwrap_or_default();
        let conversation_id = extract_text(&doc, fields.conversation_id)
            .and_then(|id| Uuid::parse_str(&id).ok())
            .unwrap_or_default();

        let created_at = extract_i64(&doc, fields.created_at)
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.with_timezone(&Utc));

        let conv_created_at = extract_i64(&doc, fields.conv_created_at)
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));

        let conv_updated_at = extract_i64(&doc, fields.conv_updated_at)
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.with_timezone(&Utc));

        hits.push(SearchHit {
            message_id,
            conversation_id,
            message_idx: i32::try_from(extract_i64(&doc, fields.message_idx).unwrap_or_default())
                .unwrap_or_default(),
            role: MessageRole::from(
                extract_text(&doc, fields.role)
                    .unwrap_or_else(|| "assistant".to_string())
                    .as_str(),
            ),
            content,
            snippet,
            created_at,
            conv_created_at,
            conv_updated_at,
            score,
            source_id: extract_text(&doc, fields.source_id).unwrap_or_default(),
            external_id: extract_text(&doc, fields.external_id),
            title: extract_text(&doc, fields.title),
            workspace: extract_text(&doc, fields.workspace),
            source_adapter: extract_text(&doc, fields.source_adapter).unwrap_or_default(),
            source_path: extract_text(&doc, fields.source_path),
            host: None,
        });
    }

    Ok(hits)
}
