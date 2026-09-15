mod app;
mod channels;
mod commands;
mod config;
mod contacts;
mod event;
mod logs;
mod radio;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use config::{Cli, Config};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use event::{AppEvent, RadioCmd};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load(Cli::parse())?;
    std::fs::create_dir_all(&cfg.data_dir)?;
    let file = tracing_appender::rolling::never(&cfg.data_dir, "meshirc.log");
    let (writer, _guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(writer)
        .with_ansi(false)
        .init();

    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);
    let (radio_tx, radio_rx) = mpsc::channel::<RadioCmd>(64);
    let mut app = App::new(cfg.clone(), radio_tx.clone())?;

    radio::spawn(cfg.port.clone(), cfg.baud, tx.clone(), radio_rx);
    event::spawn_terminal_task(tx.clone());
    event::spawn_tick_task(tx);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app, &mut rx).await;
    let _ = radio_tx.send(RadioCmd::Shutdown).await;
    restore_terminal(&mut terminal)?;
    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    rx: &mut mpsc::Receiver<AppEvent>,
) -> Result<()> {
    terminal.draw(|f| ui::draw(f, app))?;
    while let Some(ev) = rx.recv().await {
        app.handle(ev);
        while let Ok(ev) = rx.try_recv() {
            app.handle(ev);
        }
        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, app))?;
    }
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let _ = execute!(stdout, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags, DisableBracketedPaste, LeaveAlternateScreen);
        hook(info);
    }));
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(terminal.backend_mut(), DisableBracketedPaste, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
