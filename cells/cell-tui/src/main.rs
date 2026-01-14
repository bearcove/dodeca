//! Dodeca TUI cell (cell-tui)
//!
//! This cell implements TuiDisplay service - the host calls us to push updates.
//! We call HostService::send_command() to send commands back.

use std::collections::VecDeque;
use std::io::stdout;
use std::time::Duration;

use color_eyre::Result;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use roam::session::ConnectionHandle;
use roam_shm::driver::establish_guest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;
use tokio::sync::mpsc;

use cell_host_proto::{HostServiceClient, ReadyMsg, ServerCommand};
use cell_tui_proto::{
    BindMode, BuildProgress, EventKind, LogEvent, LogLevel, ServerStatus, TuiDisplay,
    TuiDisplayDispatcher,
};

mod theme;

/// Maximum number of events to keep in buffer
const MAX_EVENTS: usize = 100;

/// Preset log filter expressions (cycled with 'f' key)
/// Note: The first preset matches DEFAULT_TRACING_FILTER in dodeca/src/logging.rs
const FILTER_PRESETS: &[&str] = &[
    "warn,ddc=info",            // Quiet deps, info for dodeca (default)
    "warn,ddc=debug",           // Quiet deps, debug for dodeca
    "warn,ddc=trace",           // Quiet deps, trace for dodeca
    "info",                     // Info level for everything
    "debug",                    // Debug everything
    "warn,hyper=off,tower=off", // Suppress HTTP noise
];

/// Format bytes as compact human-readable size
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// TUI application state
struct TuiApp {
    progress: BuildProgress,
    server_status: ServerStatus,
    event_buffer: VecDeque<LogEvent>,
    command_tx: mpsc::UnboundedSender<ServerCommand>,
    show_help: bool,
    should_quit: bool,
    filter_preset_index: usize,
    /// Text input mode for custom filter
    filter_input: Option<String>,
}

impl TuiApp {
    fn new(command_tx: mpsc::UnboundedSender<ServerCommand>) -> Self {
        Self {
            progress: BuildProgress::default(),
            server_status: ServerStatus::default(),
            event_buffer: VecDeque::with_capacity(MAX_EVENTS),
            command_tx,
            show_help: false,
            should_quit: false,
            filter_preset_index: 0,
            filter_input: None,
        }
    }

    fn push_event(&mut self, event: LogEvent) {
        self.event_buffer.push_back(event);
        if self.event_buffer.len() > MAX_EVENTS {
            self.event_buffer.pop_front();
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: crossterm::event::KeyModifiers) {
        // Handle filter input mode separately
        if let Some(ref mut input) = self.filter_input {
            match code {
                KeyCode::Enter => {
                    // Apply the filter
                    let filter = std::mem::take(input);
                    self.filter_input = None;
                    let _ = self.command_tx.send(ServerCommand::SetLogFilter { filter });
                }
                KeyCode::Esc => {
                    // Cancel input
                    self.filter_input = None;
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => {
                    input.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('c') if modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                // Send exit command to host - host will shut down, we die with it
                let _ = self.command_tx.send(ServerCommand::Exit);
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    // Send exit command to host - host will shut down, we die with it
                    let _ = self.command_tx.send(ServerCommand::Exit);
                }
            }
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('o') => {
                if let Some(url) = self.server_status.urls.first()
                    && let Err(e) = open::that(url)
                {
                    eprintln!("Failed to open browser: {e}");
                }
            }
            KeyCode::Char('p') => {
                let cmd = match self.server_status.bind_mode {
                    BindMode::Local => ServerCommand::GoPublic,
                    BindMode::Lan => ServerCommand::GoLocal,
                };
                let _ = self.command_tx.send(cmd);
            }
            KeyCode::Char('d') => {
                let _ = self.command_tx.send(ServerCommand::TogglePicanteDebug);
            }
            KeyCode::Char('l') => {
                let _ = self.command_tx.send(ServerCommand::CycleLogLevel);
            }
            KeyCode::Char('f') => {
                // Cycle through preset log filters
                self.filter_preset_index = (self.filter_preset_index + 1) % FILTER_PRESETS.len();
                let filter = FILTER_PRESETS[self.filter_preset_index];
                let _ = self.command_tx.send(ServerCommand::SetLogFilter {
                    filter: filter.to_string(),
                });
            }
            KeyCode::Char('F') => {
                // Enter custom filter input mode
                self.filter_input = Some(String::new());
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame) {
        use theme::*;

        let area = frame.area();

        // Main layout
        let url_height = self.server_status.urls.len().max(1) as u16 + 2;
        let chunks = Layout::vertical([
            Constraint::Length(url_height),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

        // Server block
        let (network_icon, icon_color) = match self.server_status.bind_mode {
            BindMode::Local => ("💻", GREEN),
            BindMode::Lan => ("🏠", YELLOW),
        };
        let status = if self.server_status.is_running {
            ("●", GREEN)
        } else {
            ("○", YELLOW)
        };

        let url_lines: Vec<Line> = self
            .server_status
            .urls
            .iter()
            .map(|url| {
                Line::from(vec![
                    Span::raw("  → ").fg(CYAN),
                    Span::raw(url.clone()).fg(BLUE),
                ])
            })
            .collect();

        let server_title = Line::from(vec![
            Span::raw(" 🌐 Server "),
            Span::styled(network_icon, Style::default().fg(icon_color)),
            Span::raw(" "),
            Span::styled(status.0, Style::default().fg(status.1)),
        ]);
        let urls_widget = Paragraph::new(url_lines).block(
            Block::default()
                .title(server_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG_GUTTER)),
        );
        frame.render_widget(urls_widget, chunks[0]);

        // Build progress
        let tasks = [
            (&self.progress.parse, "📄"),
            (&self.progress.render, "🎨"),
            (&self.progress.sass, "💅"),
        ];
        let mut status_spans: Vec<Span> = Vec::new();
        for (i, (task, emoji)) in tasks.iter().enumerate() {
            if i > 0 {
                status_spans.push(Span::raw("  ").fg(FG_DARK));
            }
            let (color, symbol) = match task.status {
                TaskStatus::Pending => (FG_DARK, "○"),
                TaskStatus::Running => (CYAN, "◐"),
                TaskStatus::Done => (GREEN, "✓"),
                TaskStatus::Error => (RED, "✗"),
            };
            status_spans.push(Span::raw(format!("{emoji} ")).fg(FG));
            status_spans.push(Span::styled(symbol, Style::default().fg(color)));
        }
        // Cache size display
        let has_cache = self.server_status.picante_cache_size > 0
            || self.server_status.cas_cache_size > 0
            || self.server_status.code_exec_cache_size > 0;
        if has_cache {
            status_spans.push(Span::raw("  ").fg(FG_DARK));
            status_spans.push(Span::raw("💾 ").fg(FG));
            let mut cache_parts = vec![
                format_size(self.server_status.picante_cache_size),
                format_size(self.server_status.cas_cache_size),
            ];
            if self.server_status.code_exec_cache_size > 0 {
                cache_parts.push(format_size(self.server_status.code_exec_cache_size));
            }
            status_spans.push(Span::raw(cache_parts.join("+")).fg(FG_DARK));
        }
        let progress_widget = Paragraph::new(Line::from(status_spans)).block(
            Block::default()
                .title(" 🔨 Status ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG_GUTTER)),
        );
        frame.render_widget(progress_widget, chunks[1]);

        // Events log
        let max_events = (chunks[2].height.saturating_sub(2)) as usize;
        let recent_events: Vec<Line> = self
            .event_buffer
            .iter()
            .rev()
            .take(max_events)
            .rev()
            .map(|e| {
                let (symbol, symbol_color) = match e.level {
                    LogLevel::Error => ("✗", RED),
                    LogLevel::Warn => (event_symbol(&e.kind), YELLOW),
                    _ => (event_symbol(&e.kind), event_color(&e.kind)),
                };

                Line::from(vec![
                    Span::styled(format!("{} ", symbol), Style::default().fg(symbol_color)),
                    Span::raw(&e.message).fg(event_color(&e.kind)),
                ])
            })
            .collect();
        let events_widget = Paragraph::new(recent_events).block(
            Block::default()
                .title(" 📋 Activity ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG_GUTTER)),
        );
        frame.render_widget(events_widget, chunks[2]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::raw("?").fg(YELLOW),
            Span::raw(" help  ").fg(FG_DARK),
            Span::raw("o").fg(YELLOW),
            Span::raw(" open  ").fg(FG_DARK),
            Span::raw("p").fg(YELLOW),
            Span::raw(" public  ").fg(FG_DARK),
            Span::raw("d").fg(YELLOW),
            Span::raw(" debug  ").fg(FG_DARK),
            Span::raw("l").fg(YELLOW),
            Span::raw(" level  ").fg(FG_DARK),
            Span::raw("f").fg(YELLOW),
            Span::raw(" filter  ").fg(FG_DARK),
            Span::raw("q").fg(YELLOW),
            Span::raw(" quit").fg(FG_DARK),
        ]))
        .style(Style::default().fg(FG_DARK));
        frame.render_widget(footer, chunks[3]);

        // Filter input overlay
        if self.filter_input.is_some() {
            self.draw_filter_input(frame, area);
        }

        // Help overlay
        if self.show_help {
            self.draw_help_overlay(frame, area);
        }
    }

    fn draw_filter_input(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use theme::*;

        let input = self.filter_input.as_deref().unwrap_or("");

        let input_width = 50u16;
        let input_height = 5u16;
        let x = area.width.saturating_sub(input_width) / 2;
        let y = area.height.saturating_sub(input_height) / 2;
        let input_area = ratatui::layout::Rect::new(
            x,
            y,
            input_width.min(area.width),
            input_height.min(area.height),
        );

        frame.render_widget(Clear, input_area);

        let input_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  ").fg(FG),
                Span::raw(input).fg(CYAN),
                Span::raw("▌").fg(YELLOW), // cursor
            ]),
            Line::from(vec![
                Span::raw("  Enter").fg(YELLOW),
                Span::raw(" apply  ").fg(FG_DARK),
                Span::raw("Esc").fg(YELLOW),
                Span::raw(" cancel").fg(FG_DARK),
            ]),
        ];

        let input_widget = Paragraph::new(input_lines)
            .block(
                Block::default()
                    .title(" 🔍 Log Filter (RUST_LOG syntax) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(CYAN)),
            )
            .style(Style::default().bg(BG_DARK));

        frame.render_widget(input_widget, input_area);
    }

    fn draw_help_overlay(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use theme::*;

        let help_width = 40u16;
        let help_height = 15u16;
        let x = area.width.saturating_sub(help_width) / 2;
        let y = area.height.saturating_sub(help_height) / 2;
        let help_area = ratatui::layout::Rect::new(
            x,
            y,
            help_width.min(area.width),
            help_height.min(area.height),
        );

        frame.render_widget(Clear, help_area);

        let help_text = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  ?").fg(YELLOW),
                Span::raw("      Toggle this help").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  o").fg(YELLOW),
                Span::raw("      Open first URL in browser").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  p").fg(YELLOW),
                Span::raw("      Toggle public/local mode").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  d").fg(YELLOW),
                Span::raw("      Toggle picante debug logs").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  l").fg(YELLOW),
                Span::raw("      Cycle log level").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  f").fg(YELLOW),
                Span::raw("      Cycle log filter presets").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  F").fg(YELLOW),
                Span::raw("      Enter custom log filter").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  q").fg(YELLOW),
                Span::raw("      Quit / close panel").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  Ctrl+C").fg(YELLOW),
                Span::raw(" Force quit").fg(FG),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("  💻").fg(GREEN),
                Span::raw(" = localhost only").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  🏠").fg(YELLOW),
                Span::raw(" = LAN (home network)").fg(FG),
            ]),
        ];

        let help_widget = Paragraph::new(help_text)
            .block(
                Block::default()
                    .title(" ❓ Help ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(CYAN)),
            )
            .style(Style::default().bg(BG_DARK));

        frame.render_widget(help_widget, help_area);
    }
}

fn event_symbol(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Http { status } => {
            if *status >= 500 {
                "⚠"
            } else if *status >= 400 {
                "✗"
            } else if *status >= 300 {
                "↪"
            } else {
                "→"
            }
        }
        EventKind::FileChange => "📝",
        EventKind::Reload => "🔄",
        EventKind::Patch => "✨",
        EventKind::Search => "🔍",
        EventKind::Server => "🌐",
        EventKind::Build => "🔨",
        EventKind::Generic => "•",
    }
}

fn event_color(kind: &EventKind) -> Color {
    use theme::*;
    match kind {
        EventKind::Http { status } => http_status_color(*status),
        EventKind::FileChange => ORANGE,
        EventKind::Reload => YELLOW,
        EventKind::Patch => GREEN,
        EventKind::Search => CYAN,
        EventKind::Server => BLUE,
        EventKind::Build => PURPLE,
        EventKind::Generic => FG_DARK,
    }
}

use cell_tui_proto::TaskStatus;

/// Initialize terminal for TUI
fn init_terminal() -> Result<DefaultTerminal> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let terminal = ratatui::init();
    Ok(terminal)
}

/// Restore terminal to normal state
fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    ratatui::restore();
    Ok(())
}

/// TuiDisplay implementation - receives updates from host
#[derive(Clone)]
struct TuiDisplayImpl {
    progress_tx: mpsc::UnboundedSender<BuildProgress>,
    event_tx: mpsc::UnboundedSender<LogEvent>,
    status_tx: mpsc::UnboundedSender<ServerStatus>,
}

impl TuiDisplay for TuiDisplayImpl {
    async fn update_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send(progress);
    }

    async fn push_event(&self, event: LogEvent) {
        let _ = self.event_tx.send(event);
    }

    async fn update_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send(status);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = SpawnArgs::from_env()?;
    let transport = ShmGuestTransport::from_spawn_args(&args)?;

    // Channels for receiving updates from host (via TuiDisplay RPC)
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (status_tx, status_rx) = mpsc::unbounded_channel();

    // TuiDisplay service - host calls this to push updates
    let display_impl = TuiDisplayImpl {
        progress_tx,
        event_tx,
        status_tx,
    };
    let dispatcher = TuiDisplayDispatcher::new(display_impl);
    let (handle, driver) = establish_guest(transport, dispatcher);

    // Spawn driver in background - must run before ready() to process RPC
    let driver_handle = tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            eprintln!("Driver error: {:?}", e);
        }
    });

    // Signal readiness to host
    let host = HostServiceClient::new(handle.clone());
    host.ready(ReadyMsg {
        peer_id: args.peer_id.get() as u16,
        cell_name: "tui".to_string(),
        pid: Some(std::process::id()),
        version: None,
        features: vec![],
    })
    .await?;

    // Run the TUI loop - when it exits, terminate the cell process
    let result = run_tui(handle, progress_rx, event_rx, status_rx).await;
    if let Err(e) = &result {
        eprintln!("TUI error: {e}");
    }

    // TUI exited (user pressed 'q'), shut down
    drop(driver_handle);
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

async fn run_tui(
    handle: ConnectionHandle,
    mut progress_rx: mpsc::UnboundedReceiver<BuildProgress>,
    mut events_rx: mpsc::UnboundedReceiver<LogEvent>,
    mut status_rx: mpsc::UnboundedReceiver<ServerStatus>,
) -> Result<()> {
    // Create host client (for sending commands back)
    let client = HostServiceClient::new(handle);

    // Channel for sending commands
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<ServerCommand>();

    // Channel for key events (read on a blocking thread so we don't stall the RPC runtime)
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();

    // Initialize terminal
    let mut terminal = init_terminal()?;

    // Create app state
    let mut app = TuiApp::new(command_tx);

    // Spawn a blocking reader for keyboard input.
    // Exits automatically when the receiver is dropped.
    std::thread::spawn(move || {
        loop {
            if key_tx.is_closed() {
                break;
            }
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                        if key_tx.send(key).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Main event loop
    // Updates come via TuiDisplay RPC (host calls us)
    // Commands go via HostService::send_command (we call host)
    loop {
        tokio::select! {
            _ = tick.tick() => {}

            Some(key) = key_rx.recv() => {
                app.handle_key(key.code, key.modifiers);
            }

            Some(cmd) = command_rx.recv() => {
                let _ = client.send_command(cmd).await;
            }

            Some(progress) = progress_rx.recv() => {
                app.progress = progress;
            }

            Some(event) = events_rx.recv() => {
                app.push_event(event);
            }

            Some(status) = status_rx.recv() => {
                app.server_status = status;
            }
        }

        // Draw after handling any update (including tick) so we stay responsive to keys
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit {
            break;
        }
    }

    // Cleanup
    restore_terminal()?;

    Ok(())
}
