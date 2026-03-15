pub mod app;
pub mod ui;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};

use crate::config::LadderConfig;
use crate::env::RuntimeState;

/// Entry point for TUI mode
pub async fn run() -> Result<()> {
    let cfg = LadderConfig::load()?;
    let state = RuntimeState::load()?;

    if !state.running {
        anyhow::bail!("Proxy is not running. Start with `ladder start` first.");
    }

    let mut app = app::App::new(cfg, state)?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initial data load
    app.refresh_nodes().await;

    let tick_rate = Duration::from_millis(250);
    let refresh_interval = Duration::from_secs(5);

    let result = run_event_loop(&mut terminal, &mut app, tick_rate, refresh_interval).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
    tick_rate: Duration,
    refresh_interval: Duration,
) -> Result<()> {
    loop {
        // Draw frame
        terminal.draw(|f| ui::render(f, app))?;

        // Process any pending background task messages (non-blocking)
        app.process_bg_messages().await;

        // Auto-refresh node list (only when idle)
        if app.last_refresh.elapsed() >= refresh_interval && app.mode == app::AppMode::Normal {
            app.refresh_nodes().await;
        }

        // Tick status message expiry
        app.tick_status();

        // Poll for events with timeout
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                // Only handle press events (not repeat/release)
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(app, key.code).await;
            }
        }

        if app.should_quit {
            // Cancel any running background task before quitting
            app.cancel_current();
            break;
        }
    }
    Ok(())
}

async fn handle_key(app: &mut app::App, key: KeyCode) {
    // During background operations: only allow cancel (Esc/q) and navigation
    if app.mode != app::AppMode::Normal {
        match key {
            KeyCode::Esc => {
                app.cancel_current();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                app.cancel_current();
                app.should_quit = true;
            }
            // Allow navigation during background ops
            KeyCode::Up | KeyCode::Char('k') => app.cursor_up(),
            KeyCode::Down | KeyCode::Char('j') => app.cursor_down(),
            KeyCode::Tab => app.next_tab(),
            KeyCode::BackTab => app.prev_tab(),
            KeyCode::Left | KeyCode::Char('h') => app.prev_tab(),
            KeyCode::Right | KeyCode::Char('l') => app.next_tab(),
            _ => {}
        }
        return;
    }

    match key {
        // Quit - proxy keeps running
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => app.cursor_up(),
        KeyCode::Down | KeyCode::Char('j') => app.cursor_down(),

        // Tab switching
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),
        KeyCode::Left | KeyCode::Char('h') => app.prev_tab(),
        KeyCode::Right | KeyCode::Char('l') => app.next_tab(),

        // Select node
        KeyCode::Enter => app.select_current().await,

        // Speed test (non-blocking)
        KeyCode::Char('t') | KeyCode::Char('T') => app.run_speed_test(),

        // Auto-best (non-blocking)
        KeyCode::Char('a') | KeyCode::Char('A') => app.auto_best(),

        // Update providers (non-blocking)
        KeyCode::Char('u') | KeyCode::Char('U') => app.update_providers(),

        // Manual refresh
        KeyCode::Char('r') | KeyCode::Char('R') => app.refresh_nodes().await,

        _ => {}
    }
}
