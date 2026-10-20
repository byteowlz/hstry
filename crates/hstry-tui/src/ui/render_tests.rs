//! Render the real `App` into an in-memory terminal and assert on the frame:
//! layout, rows, query bar and action bar, at wide and 80×24 sizes.

use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

use hstry_core::{
    Config, Database, db::ListConversationsOptions, ingest::ingest_batch, models::Source,
};

use crate::keys::{self, Flow};
use crate::state::{App, Focus, Mode, Scope};

struct Fixture {
    _dir: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    app: App,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let config = Config {
        database: dir.path().join("history.db"),
        ..Default::default()
    };
    let app = rt.block_on(async {
        let db = Database::open(&config.database).await.expect("db");
        for (id, adapter) in [("claude-code", "claude-code"), ("codex", "codex")] {
            db.upsert_source(&Source {
                id: id.into(),
                adapter: adapter.into(),
                path: Some(format!("/home/me/.{adapter}")),
                last_sync_at: None,
                config: json!({}),
            })
            .await
            .expect("source");
        }
        let session = |id: &str, title: &str, ws: &str, at: i64, tool_output: &str| {
            serde_json::from_value(json!({
                "externalId": id,
                "title": title,
                "workspace": ws,
                "createdAt": at,
                "updatedAt": at,
                "metadata": {"file": format!("/home/me/sessions/{id}.jsonl")},
                "messages": [
                    {"role": "user", "content": format!("Please {title}"), "createdAt": at},
                    {"role": "assistant", "content": "", "createdAt": at + 1000, "parts": [
                        {"id": "p1", "type": "thinking", "text": "Read the code first."},
                        {"id": "p2", "type": "tool_call", "toolCallId": "t1", "name": "Bash", "input": {"command": "rg -n backoff src/"}}
                    ]},
                    {"role": "tool", "content": tool_output, "createdAt": at + 2000, "parts": [
                        {"id": "p3", "type": "tool_result", "toolCallId": "t1", "output": tool_output}
                    ]},
                    {"role": "assistant", "content": format!("Done: {title}."), "createdAt": at + 3000}
                ]
            }))
            .expect("conversation")
        };
        ingest_batch(
            &db,
            "claude-code",
            vec![
                session("a", "Fix flaky retry test in the sync loop", "/work/hstry", 1_760_000_000_000, "src/sync.rs:41: let delay = backoff(attempt);"),
                session("b", "Write release notes for 0.5.28", "/work/hstry", 1_759_000_000_000, "CHANGELOG.md"),
            ],
        )
        .await
        .expect("ingest");
        ingest_batch(
            &db,
            "codex",
            vec![session("c", "Refactor auth middleware to use the token store", "/work/oqto", 1_758_000_000_000, "test result: ok")],
        )
        .await
        .expect("ingest");
        let ids: Vec<_> = db
            .list_conversations(ListConversationsOptions::default())
            .await
            .expect("list")
            .into_iter()
            .map(|c| c.id)
            .collect();
        db.rebuild_conversation_summaries(&ids).await.expect("summaries");
        let sources = db.list_sources().await.expect("sources");
        let summaries = db
            .list_conversation_summaries(ListConversationsOptions::default())
            .await
            .expect("summaries");
        App::new(config, db, sources, summaries, ratatui_image::picker::Picker::halfblocks())
    });
    Fixture { _dir: dir, rt, app }
}

fn render(fx: &mut Fixture, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    fx.app.load_messages(&fx.rt);
    terminal
        .draw(|f| super::draw(f, &mut fx.app))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn press(fx: &mut Fixture, code: crossterm::event::KeyCode) -> Flow {
    let key = crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
    keys::handle_key(&mut fx.app, &key, &fx.rt)
}

fn type_text(fx: &mut Fixture, text: &str) {
    for ch in text.chars() {
        press(fx, crossterm::event::KeyCode::Char(ch));
    }
}

fn frame_text(lines: &[String]) -> String {
    lines.join("\n")
}

#[test]
fn wide_layout_shows_list_transcript_and_actions() {
    let mut fx = fixture();
    let lines = render(&mut fx, 140, 30);
    let text = frame_text(&lines);
    // Query bar: brand, placeholder, scope switch, count.
    assert!(lines[0].starts_with(" hstry "), "{}", lines[0]);
    assert!(
        lines[0].contains("press / to find a session"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("sessions") && lines[0].contains("content"),
        "{}",
        lines[0]
    );
    assert!(lines[0].trim_end().ends_with("3 sessions"), "{}", lines[0]);
    // Newest session first, as a two-line row: title then meta.
    let title_row = lines
        .iter()
        .position(|l| l.contains("▌") && l.contains("Fix flaky retry test"))
        .expect("selected title row");
    assert!(
        lines[title_row + 1].contains("claude-code · hstry"),
        "{}",
        lines[title_row + 1]
    );
    assert!(
        lines[title_row + 1].contains("4 msgs"),
        "{}",
        lines[title_row + 1]
    );
    // Transcript: thinking, tool call and tool output are distinct, and the
    // tool output is not attributed to the user.
    assert!(text.contains("∴ Read the code first."), "{text}");
    assert!(text.contains("⚙ Bash  rg -n backoff src/"), "{text}");
    assert!(text.contains("└ src/sync.rs:41"), "{text}");
    assert_eq!(text.matches("● you").count(), 1, "{text}");
    // Action bar names the verbs for the selection.
    let bar = lines.last().expect("bar");
    for verb in ["read", "open in…", "copy", "copy id", "copy path", "export"] {
        assert!(bar.contains(verb), "missing {verb}: {bar}");
    }
}

#[test]
fn small_terminal_shows_one_pane_with_readable_titles() {
    let mut fx = fixture();
    let lines = render(&mut fx, 80, 24);
    let text = frame_text(&lines);
    assert!(fx.app.narrow);
    assert!(
        text.contains("Fix flaky retry test in the sync loop"),
        "title cut too early:\n{text}"
    );
    assert!(
        !text.contains("● you"),
        "transcript must not share the screen at 80 cols:\n{text}"
    );

    press(&mut fx, crossterm::event::KeyCode::Enter);
    let read = frame_text(&render(&mut fx, 80, 24));
    assert_eq!(fx.app.focus, Focus::Transcript);
    assert!(read.contains("● you"), "{read}");
    assert!(read.contains("Esc back"), "{read}");

    press(&mut fx, crossterm::event::KeyCode::Esc);
    assert_eq!(fx.app.focus, Focus::List);
}

#[test]
fn typing_filters_live_and_selects_the_best_match() {
    let mut fx = fixture();
    press(&mut fx, crossterm::event::KeyCode::Char('/'));
    assert_eq!(fx.app.mode, Mode::Query);
    type_text(&mut fx, "auth");
    let lines = render(&mut fx, 140, 30);
    assert!(lines[0].contains("auth"), "{}", lines[0]);
    assert!(lines[0].contains("1 of 3"), "{}", lines[0]);
    assert_eq!(fx.app.visible.len(), 1);
    assert_eq!(
        fx.app
            .cursor_session()
            .map(|s| s.title().to_string())
            .as_deref(),
        Some("Refactor auth middleware to use the token store")
    );
    // Esc clears and returns to the full list.
    press(&mut fx, crossterm::event::KeyCode::Esc);
    assert_eq!(fx.app.mode, Mode::Normal);
    assert_eq!(fx.app.visible.len(), 3);
}

#[test]
fn metadata_terms_narrow_the_list() {
    let mut fx = fixture();
    press(&mut fx, crossterm::event::KeyCode::Char('/'));
    type_text(&mut fx, "codex");
    assert_eq!(fx.app.visible.len(), 1);
    press(&mut fx, crossterm::event::KeyCode::Char('u'));
    // "codexu" matches nothing: the list empties and says so.
    let text = frame_text(&render(&mut fx, 140, 30));
    assert!(
        fx.app.visible.is_empty(),
        "{:?}",
        fx.app
            .visible
            .iter()
            .map(|v| fx.app.sessions[v.idx].title().to_string())
            .collect::<Vec<_>>()
    );
    assert!(text.contains("Nothing matches"), "{text}");
}

#[test]
fn tab_switches_to_content_search_grouped_per_session() {
    let mut fx = fixture();
    press(&mut fx, crossterm::event::KeyCode::Char('/'));
    press(&mut fx, crossterm::event::KeyCode::Tab);
    assert_eq!(fx.app.scope, Scope::Content);
    type_text(&mut fx, "backoff");
    // Enter runs the debounced search immediately.
    press(&mut fx, crossterm::event::KeyCode::Enter);
    let text = frame_text(&render(&mut fx, 140, 30));
    assert_eq!(
        fx.app.content_results.len(),
        1,
        "one session mentions backoff"
    );
    assert!(
        text.contains("1 sessions match") || text.contains("1 session"),
        "{text}"
    );
    assert!(text.contains("Fix flaky retry test"), "{text}");
    assert!(text.contains("hit"), "{text}");
}

#[test]
fn details_toggle_expands_collapsed_output() {
    let mut fx = fixture();
    render(&mut fx, 140, 30);
    let collapsed = fx.app.chat_lines.len();
    press(&mut fx, crossterm::event::KeyCode::Char('t'));
    assert!(fx.app.expand_details);
    render(&mut fx, 140, 30);
    assert!(fx.app.chat_lines.len() >= collapsed);
}

#[test]
fn open_in_offers_configured_agents() {
    let mut fx = fixture();
    render(&mut fx, 140, 30);
    press(&mut fx, crossterm::event::KeyCode::Char('o'));
    let text = frame_text(&render(&mut fx, 140, 30));
    assert!(text.contains("open in"), "{text}");
    assert!(text.contains("pi"), "{text}");
    press(&mut fx, crossterm::event::KeyCode::Esc);
    assert!(fx.app.overlay.is_none());
}

#[test]
fn export_writes_a_markdown_file() {
    let mut fx = fixture();
    let out = tempfile::tempdir().expect("out");
    let previous = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(out.path()).expect("chdir");
    render(&mut fx, 140, 30);
    press(&mut fx, crossterm::event::KeyCode::Char('e'));
    press(&mut fx, crossterm::event::KeyCode::Enter);
    std::env::set_current_dir(previous).expect("chdir back");
    let files: Vec<_> = std::fs::read_dir(out.path())
        .expect("read")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0]
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        name.starts_with("hstry-fix-flaky-retry-test") && name.ends_with(".md"),
        "{name}"
    );
    let body = std::fs::read_to_string(&files[0]).expect("body");
    assert!(
        body.contains("# Fix flaky retry test in the sync loop"),
        "{body}"
    );
}
