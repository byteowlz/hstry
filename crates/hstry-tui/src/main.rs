#![allow(clippy::print_stdout, clippy::print_stderr)]

//! hstry TUI: read past agent sessions fast, jump back in with a coding agent.

mod actions;
mod clipboard;
mod fuzzy;
mod images;
mod keys;
mod markdown;
mod state;
mod theme;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use hstry_core::{Config, Database, db::ListConversationsOptions};

use keys::Flow;
use state::{App, Mode};

#[derive(Debug, Parser)]
#[command(author, version, about = "TUI interface for hstry chat history")]
struct Cli {
    #[command(flatten)]
    common: CommonOpts,

    /// Start with this query in the find bar
    #[arg(value_name = "QUERY")]
    query: Option<String>,

    /// Search message content instead of session titles
    #[arg(long)]
    content: bool,
}

#[derive(Debug, Clone, Args)]
struct CommonOpts {
    /// Override the config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

/// RAII terminal teardown: restore on drop, even on panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .common
        .config
        .unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    let rt = tokio::runtime::Runtime::new()?;
    let db = rt.block_on(Database::open(&config.database))?;
    let sources = rt.block_on(db.list_sources())?;
    let summaries = rt.block_on(db.list_conversation_summaries(ListConversationsOptions {
        limit: None,
        ..Default::default()
    }))?;

    // Query the background before raw mode and the alternate screen.
    theme::set_light(theme::detect_light());
    let picker = images::detect_picker();

    let mut app = App::new(config, db, sources, summaries, picker);
    if cli.content {
        app.scope = state::Scope::Content;
    }
    if let Some(query) = cli.query.filter(|q| !q.trim().is_empty()) {
        app.query_cursor = query.chars().count();
        app.query = query;
        app.mode = Mode::Query;
        app.query_edited();
        if app.scope == state::Scope::Content {
            app.perform_content_search(&app.query.clone(), &rt);
        }
    }

    let result = {
        let _guard = TerminalGuard::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        run_app(&mut terminal, &mut app, &rt)
    };

    let pending = app.pending_resume.take();
    rt.block_on(app.db.close());
    result?;

    if let Some((id, agent)) = pending {
        let mut cmd = std::process::Command::new("hstry");
        cmd.arg("resume").arg(id.to_string());
        if let Some(agent) = agent {
            cmd.args(["--agent", &agent]);
        }
        match cmd.status() {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("hstry resume exited with {s}"),
            Err(e) => eprintln!("Failed to launch `hstry resume`: {e}"),
        }
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rt: &tokio::runtime::Runtime,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    app.load_messages(rt);

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(60))? {
            // Idle: run debounced work (content search) and let status fade.
            app.tick(rt);
            continue;
        }
        match event::read()? {
            Event::Key(key)
                if key.kind == KeyEventKind::Press
                    && keys::handle_key(app, &key, rt) == Flow::Quit =>
            {
                return Ok(());
            }
            Event::Mouse(mouse) => keys::handle_mouse(app, mouse.kind, mouse.column, mouse.row, rt),
            _ => {}
        }
    }
}
