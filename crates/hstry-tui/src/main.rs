use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use rust_core::{AppConfig, AppPaths};

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover(cli.common.config)?;
    let config = AppConfig::load(&paths, false)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

#[derive(Debug, Parser)]
#[command(author, version, about = "TUI interface for rust-workspace")]
struct Cli {
    #[command(flatten)]
    common: CommonOpts,
}

#[derive(Debug, Clone, Args)]
struct CommonOpts {
    /// Override the config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

#[derive(Debug)]
enum AppMode {
    Normal,
    Help,
}

struct App {
    config: AppConfig,
    mode: AppMode,
    selected_index: usize,
    items: Vec<String>,
    status_message: String,
}

impl App {
    fn new(config: AppConfig) -> Self {
        Self {
            config,
            mode: AppMode::Normal,
            selected_index: 0,
            items: vec![
                "Item 1".to_string(),
                "Item 2".to_string(),
                "Item 3".to_string(),
                "Item 4".to_string(),
                "Item 5".to_string(),
            ],
            status_message: "Press ? for help, q to quit".to_string(),
        }
    }

    fn next(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.items.len();
        }
    }

    fn previous(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = self
                .selected_index
                .checked_sub(1)
                .unwrap_or(self.items.len() - 1);
        }
    }
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match &app.mode {
                    AppMode::Normal => match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('?') => app.mode = AppMode::Help,
                        KeyCode::Char('j') | KeyCode::Down => app.next(),
                        KeyCode::Char('k') | KeyCode::Up => app.previous(),
                        _ => {}
                    },
                    AppMode::Help => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                            app.mode = AppMode::Normal
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(chunks[0]);

    draw_left_pane(f, app, main_chunks[0]);
    draw_middle_pane(f, app, main_chunks[1]);
    draw_right_pane(f, app, main_chunks[2]);
    draw_status_bar(f, app, chunks[1]);

    if matches!(app.mode, AppMode::Help) {
        draw_help_overlay(f);
    }
}

fn draw_left_pane(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default().title(" Navigation ").borders(Borders::ALL);
    let items = vec![
        ListItem::new("All"),
        ListItem::new("Recent"),
        ListItem::new("Favorites"),
    ];
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_middle_pane(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().title(" Items ").borders(Borders::ALL);
    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == app.selected_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(item.as_str()).style(style)
        })
        .collect();
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_right_pane(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().title(" Details ").borders(Borders::ALL);
    let content = if app.selected_index < app.items.len() {
        format!(
            "Selected: {}\nProfile: {}",
            app.items[app.selected_index], app.config.profile
        )
    } else {
        "No item selected".to_string()
    };
    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let mode_indicator = match app.mode {
        AppMode::Normal => Span::styled(
            " NORMAL ",
            Style::default().fg(Color::Black).bg(Color::Green),
        ),
        AppMode::Help => Span::styled(
            " HELP ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
    };

    let status = Line::from(vec![
        mode_indicator,
        Span::raw(" "),
        Span::raw(&app.status_message),
    ]);

    f.render_widget(Paragraph::new(status), area);
}

fn draw_help_overlay(f: &mut Frame) {
    let area = centered_rect(60, 60, f.area());
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray));

    let help_text = vec![
        Line::from("Navigation:"),
        Line::from("  j/Down  - Move down"),
        Line::from("  k/Up    - Move up"),
        Line::from(""),
        Line::from("Actions:"),
        Line::from("  ?       - Toggle help"),
        Line::from("  q       - Quit"),
        Line::from(""),
        Line::from("Press Esc or ? to close"),
    ];

    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
