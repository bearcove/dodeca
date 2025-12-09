//! TUI for dodeca build progress using ratatui
//!
//! Shows live progress for parallel build tasks with a clean terminal UI.

#![allow(dead_code)] // Some LogEvent constructors reserved for future use

use color_eyre::Result;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::collections::VecDeque;
use std::io::stdout;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::sync::watch;

/// Format bytes as compact human-readable size
fn format_size(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Progress state for a single task
#[derive(Debug, Clone)]
pub struct TaskProgress {
    pub name: &'static str,
    pub total: usize,
    pub completed: usize,
    pub status: TaskStatus,
    pub message: Option<String>,
}

/// Status of a build task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Done,
    Error,
}

impl TaskProgress {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            total: 0,
            completed: 0,
            status: TaskStatus::Pending,
            message: None,
        }
    }

    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }

    pub fn start(&mut self, total: usize) {
        self.total = total;
        self.completed = 0;
        self.status = TaskStatus::Running;
    }

    pub fn advance(&mut self) {
        self.completed = (self.completed + 1).min(self.total);
    }

    pub fn finish(&mut self) {
        self.completed = self.total;
        self.status = TaskStatus::Done;
    }

    pub fn fail(&mut self, msg: impl Into<String>) {
        self.status = TaskStatus::Error;
        self.message = Some(msg.into());
    }
}

/// All build progress state
#[derive(Debug, Clone)]
pub struct BuildProgress {
    pub parse: TaskProgress,
    pub render: TaskProgress,
    pub sass: TaskProgress,
    pub links: TaskProgress,
    pub search: TaskProgress,
}

impl Default for BuildProgress {
    fn default() -> Self {
        Self {
            parse: TaskProgress::new("Parsing"),
            render: TaskProgress::new("Rendering"),
            sass: TaskProgress::new("Sass"),
            links: TaskProgress::new("Links"),
            search: TaskProgress::new("Search"),
        }
    }
}

/// Shared progress state for use across threads (legacy, for build mode)
pub type SharedProgress = Arc<Mutex<BuildProgress>>;

/// Create a new shared progress state (legacy, for build mode)
#[allow(dead_code)]
pub fn new_shared_progress() -> SharedProgress {
    Arc::new(Mutex::new(BuildProgress::default()))
}

// ============================================================================
// Channel-based types for serve mode (cleaner than locks)
// ============================================================================

/// Progress sender - producers call send_modify to update progress
pub type ProgressTx = watch::Sender<BuildProgress>;
/// Progress receiver - TUI reads latest progress
pub type ProgressRx = watch::Receiver<BuildProgress>;

/// Create a new progress channel
pub fn progress_channel() -> (ProgressTx, ProgressRx) {
    watch::channel(BuildProgress::default())
}

/// Server status sender
pub type ServerStatusTx = watch::Sender<ServerStatus>;
/// Server status receiver
pub type ServerStatusRx = watch::Receiver<ServerStatus>;

/// Create a new server status channel
pub fn server_status_channel() -> (ServerStatusTx, ServerStatusRx) {
    watch::channel(ServerStatus::default())
}

/// Log level for activity events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Kind of activity event (for display styling)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventKind {
    /// HTTP request (GET, POST, etc.)
    Http { status: u16 },
    /// File system change
    FileChange,
    /// Live reload triggered
    Reload,
    /// DOM patches sent
    Patch,
    /// Search index update
    Search,
    /// Server status
    Server,
    /// Salsa/build related
    Build,
    /// Generic info
    #[default]
    Generic,
}

impl EventKind {
    /// Symbol for this event kind
    pub fn symbol(&self) -> &'static str {
        match self {
            EventKind::Http { status } => {
                if *status >= 500 {
                    "⚠"  // server error
                } else if *status >= 400 {
                    "✗"  // client error
                } else if *status >= 300 {
                    "↪"  // redirect
                } else {
                    "→"  // success
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

    /// Color for this event kind (Tokyo Night palette)
    pub fn color(&self) -> Color {
        use crate::theme;
        match self {
            EventKind::Http { status } => theme::http_status_color(*status),
            EventKind::FileChange => theme::ORANGE,
            EventKind::Reload => theme::YELLOW,
            EventKind::Patch => theme::GREEN,
            EventKind::Search => theme::CYAN,
            EventKind::Server => theme::BLUE,
            EventKind::Build => theme::PURPLE,
            EventKind::Generic => theme::FG_DARK,
        }
    }
}

/// A log event with level, kind, and message
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub level: LogLevel,
    pub kind: EventKind,
    pub message: String,
}

impl LogEvent {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    pub fn warn(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warn,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Error,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    pub fn with_kind(mut self, kind: EventKind) -> Self {
        self.kind = kind;
        self
    }

    // Convenience constructors for specific event types
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self {
            level: if status >= 400 { LogLevel::Warn } else { LogLevel::Info },
            kind: EventKind::Http { status },
            message: message.into(),
        }
    }

    pub fn file_change(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::FileChange,
            message: message.into(),
        }
    }

    pub fn reload(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Reload,
            message: message.into(),
        }
    }

    pub fn patch(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Patch,
            message: message.into(),
        }
    }

    pub fn search(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Search,
            message: message.into(),
        }
    }

    pub fn server(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Server,
            message: message.into(),
        }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Build,
            message: message.into(),
        }
    }
}

/// Event sender - multiple producers can clone and send
pub type EventTx = mpsc::Sender<LogEvent>;
/// Event receiver - TUI drains events
pub type EventRx = mpsc::Receiver<LogEvent>;

/// Create a new event channel
pub fn event_channel() -> (EventTx, EventRx) {
    mpsc::channel()
}

/// Helper to update progress - works with either SharedProgress or ProgressTx
pub enum ProgressReporter {
    /// Legacy mutex-based (for build command)
    Shared(SharedProgress),
    /// Channel-based (for serve mode)
    Channel(ProgressTx),
}

impl ProgressReporter {
    /// Update progress with a closure
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut BuildProgress),
    {
        match self {
            ProgressReporter::Shared(p) => {
                let mut prog = p.lock().unwrap();
                f(&mut prog);
            }
            ProgressReporter::Channel(tx) => {
                tx.send_modify(f);
            }
        }
    }
}

/// Initialize terminal for TUI
pub fn init_terminal() -> Result<DefaultTerminal> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let terminal = ratatui::init();
    Ok(terminal)
}

/// Restore terminal to normal state
pub fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    ratatui::restore();
    Ok(())
}

/// Get all LAN (private) IPv4 addresses from network interfaces
pub fn get_lan_ips() -> Vec<std::net::Ipv4Addr> {
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        interfaces
            .into_iter()
            .filter_map(|iface| {
                if let if_addrs::IfAddr::V4(addr) = iface.addr {
                    let ip = addr.ip;
                    // Include private IPs but not loopback
                    if ip.is_private() || ip.is_link_local() {
                        Some(ip)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    }
}

/// Server binding mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindMode {
    /// Local only (127.0.0.1) - shown with 💻
    #[default]
    Local,
    /// LAN interfaces (private IPs) - shown with 🏠
    Lan,
}

/// Server status for serve mode TUI
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub urls: Vec<String>,
    pub is_running: bool,
    pub bind_mode: BindMode,
    /// Salsa cache size in bytes (dodeca.bin)
    pub salsa_cache_size: usize,
    /// CAS/image cache size in bytes (.cache directory)
    pub cas_cache_size: usize,
}

/// Command sent from TUI to server
#[derive(Debug, Clone)]
pub enum ServerCommand {
    /// Switch to LAN mode (bind to 0.0.0.0)
    GoPublic,
    /// Switch to local mode (bind to 127.0.0.1)
    GoLocal,
}

/// Serve mode TUI application state (channel-based)
pub struct ServeApp {
    progress_rx: ProgressRx,
    server_rx: ServerStatusRx,
    event_rx: EventRx,
    /// Local buffer for events (since mpsc drains)
    event_buffer: VecDeque<LogEvent>,
    command_tx: tokio::sync::mpsc::UnboundedSender<ServerCommand>,
    /// Handle for dynamically updating log filters
    filter_handle: crate::logging::FilterHandle,
    /// Whether salsa debug logging is enabled
    salsa_debug: bool,
    /// Current log level for TUI filtering
    log_level: crate::logging::TuiLogLevel,
    show_help: bool,
    should_quit: bool,
}

/// Maximum number of events to keep in the buffer
const MAX_EVENTS: usize = 100;

/// Highlight paths, URLs, HTTP methods, and timings in a log message
fn highlight_message(msg: &str, kind: EventKind) -> Vec<Span<'static>> {
    use crate::theme::{self, TokyoNight};
    use ratatui::style::Stylize;

    let mut spans = Vec::new();
    let remaining = msg.to_string();

    // For HTTP events, try to parse "METHOD /path -> STATUS in TIMEms"
    if matches!(kind, EventKind::Http { .. }) {
        // Try to match: "GET /foo/bar -> 200 in 5ms"
        if let Some(arrow_pos) = remaining.find(" -> ") {
            let before_arrow = &remaining[..arrow_pos];
            let after_arrow = &remaining[arrow_pos + 4..];

            // Parse method and path from before arrow
            if let Some(space_pos) = before_arrow.find(' ') {
                let method = &before_arrow[..space_pos];
                let path = &before_arrow[space_pos + 1..];

                spans.push(Span::raw(method.to_string()).tn_cyan());
                spans.push(Span::raw(" "));
                spans.push(Span::raw(path.to_string()).tn_path());
                spans.push(Span::raw(" → ").tn_muted());

                // Parse status and timing from after arrow
                if let Some(in_pos) = after_arrow.find(" in ") {
                    let status = &after_arrow[..in_pos];
                    let timing = &after_arrow[in_pos + 4..];

                    // Color status based on code
                    let status_color = if let Ok(code) = status.parse::<u16>() {
                        theme::http_status_color(code)
                    } else {
                        theme::FG
                    };
                    spans.push(Span::styled(status.to_string(), Style::default().fg(status_color)));
                    spans.push(Span::raw(" in ").tn_muted());
                    spans.push(Span::raw(timing.to_string()).tn_timing());
                } else {
                    spans.push(Span::raw(after_arrow.to_string()).tn_fg());
                }
                return spans;
            }
        }
    }

    // For file change events, highlight the path
    if matches!(kind, EventKind::FileChange) {
        // Common patterns: "Modified: /path" or just "/path"
        if let Some(colon_pos) = remaining.find(": ") {
            let prefix = &remaining[..colon_pos + 2];
            let path = &remaining[colon_pos + 2..];
            spans.push(Span::raw(prefix.to_string()).tn_fg_dark());
            spans.push(Span::raw(path.to_string()).tn_path());
            return spans;
        } else if remaining.starts_with('/') || remaining.contains('/') {
            spans.push(Span::raw(remaining).tn_path());
            return spans;
        }
    }

    // For reload/patch events, highlight paths
    if matches!(kind, EventKind::Reload | EventKind::Patch) {
        // Look for paths in the message
        let parts: Vec<&str> = remaining.split('/').collect();
        if parts.len() > 1 {
            // Has a path - find where it starts
            if let Some(slash_pos) = remaining.find('/') {
                let before = &remaining[..slash_pos];
                let path = &remaining[slash_pos..];
                if !before.is_empty() {
                    spans.push(Span::raw(before.to_string()).tn_fg());
                }
                spans.push(Span::raw(path.to_string()).tn_path());
                return spans;
            }
        }
    }

    // Default: just return the message with default event color
    spans.push(Span::raw(remaining).fg(kind.color()));
    spans
}

impl ServeApp {
    pub fn new(
        progress_rx: ProgressRx,
        server_rx: ServerStatusRx,
        event_rx: EventRx,
        command_tx: tokio::sync::mpsc::UnboundedSender<ServerCommand>,
        filter_handle: crate::logging::FilterHandle,
    ) -> Self {
        let salsa_debug = filter_handle.is_salsa_debug_enabled();
        let log_level = filter_handle.get_log_level();
        Self {
            progress_rx,
            server_rx,
            event_rx,
            event_buffer: VecDeque::with_capacity(MAX_EVENTS),
            command_tx,
            filter_handle,
            salsa_debug,
            log_level,
            show_help: false,
            should_quit: false,
        }
    }

    /// Run the serve TUI event loop
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_quit {
            // Drain any new events into the buffer
            self.drain_events();

            terminal.draw(|frame| self.draw(frame))?;

            // Poll for events with timeout
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('c')
                                if key
                                    .modifiers
                                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
                            {
                                self.should_quit = true
                            }
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if self.show_help {
                                    self.show_help = false;
                                } else {
                                    self.should_quit = true;
                                }
                            }
                            KeyCode::Char('?') => self.show_help = !self.show_help,
                            KeyCode::Char('p') => {
                                let current_mode = self.server_rx.borrow().bind_mode;
                                let cmd = match current_mode {
                                    BindMode::Local => ServerCommand::GoPublic,
                                    BindMode::Lan => ServerCommand::GoLocal,
                                };
                                let _ = self.command_tx.send(cmd);
                            }
                            KeyCode::Char('d') => {
                                self.salsa_debug = self.filter_handle.toggle_salsa_debug();
                                let status = if self.salsa_debug {
                                    "enabled"
                                } else {
                                    "disabled"
                                };
                                self.event_buffer
                                    .push_back(LogEvent::info(format!("Salsa debug {status}")));
                            }
                            KeyCode::Char('l') => {
                                self.log_level = self.filter_handle.cycle_log_level();
                                self.event_buffer
                                    .push_back(LogEvent::info(format!("Log level: {}", self.log_level.as_str())));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Drain events from the channel into the local buffer
    fn drain_events(&mut self) {
        // Non-blocking drain of all available events
        while let Ok(event) = self.event_rx.try_recv() {
            self.event_buffer.push_back(event);
            // Keep buffer bounded
            if self.event_buffer.len() > MAX_EVENTS {
                self.event_buffer.pop_front();
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        use crate::theme::{self, TokyoNight};

        // Read from channels (no locks!)
        let progress = self.progress_rx.borrow();
        let server = self.server_rx.borrow();

        let area = frame.area();

        // Main layout
        let url_height = server.urls.len().max(1) as u16 + 2;
        let chunks = Layout::vertical([
            Constraint::Length(url_height), // Server URLs
            Constraint::Length(3),          // Build progress (single line + borders)
            Constraint::Min(3),             // Events log
            Constraint::Length(1),          // Footer
        ])
        .split(area);

        // Server block with network icon and status in title
        let (network_icon, icon_color) = match server.bind_mode {
            BindMode::Local => ("💻", theme::GREEN),
            BindMode::Lan => ("🏠", theme::YELLOW),
        };
        let status = if server.is_running {
            ("●", theme::GREEN)
        } else {
            ("○", theme::YELLOW)
        };

        let url_lines: Vec<Line> = server
            .urls
            .iter()
            .map(|url| {
                Line::from(vec![
                    Span::raw("  → ").tn_cyan(),
                    Span::raw(url.clone()).tn_url(),
                ])
            })
            .collect();

        let server_title = Line::from(vec![
            Span::raw(" 🌐 Server "),
            Span::styled(network_icon, Style::default().fg(icon_color)),
            Span::raw(" "),
            Span::styled(status.0, Style::default().fg(status.1)),
        ]);
        let urls_widget = Paragraph::new(url_lines)
            .block(Block::default().title(server_title).borders(Borders::ALL).border_style(Style::default().fg(theme::FG_GUTTER)));
        frame.render_widget(urls_widget, chunks[0]);

        // Build progress (single-line compact version)
        let tasks = [
            (&progress.parse, "📄"),
            (&progress.render, "🎨"),
            (&progress.sass, "💅"),
        ];
        let mut status_spans: Vec<Span> = Vec::new();
        for (i, (task, emoji)) in tasks.iter().enumerate() {
            if i > 0 {
                status_spans.push(Span::raw("  ").tn_fg_dark());
            }
            let (color, symbol) = match task.status {
                TaskStatus::Pending => (theme::FG_DARK, "○"),
                TaskStatus::Running => (theme::CYAN, "◐"),
                TaskStatus::Done => (theme::GREEN, "✓"),
                TaskStatus::Error => (theme::RED, "✗"),
            };
            status_spans.push(Span::raw(format!("{emoji} ")).tn_fg());
            status_spans.push(Span::styled(symbol, Style::default().fg(color)));
        }
        // Add cache size display
        if server.salsa_cache_size > 0 || server.cas_cache_size > 0 {
            status_spans.push(Span::raw("  ").tn_fg_dark());
            status_spans.push(Span::raw("💾 ").tn_fg());
            status_spans.push(Span::raw(format!(
                "{}+{}",
                format_size(server.salsa_cache_size),
                format_size(server.cas_cache_size)
            )).tn_fg_dark());
        }
        let progress_widget = Paragraph::new(Line::from(status_spans))
            .block(Block::default().title(" 🔨 Status ").borders(Borders::ALL).border_style(Style::default().fg(theme::FG_GUTTER)));
        frame.render_widget(progress_widget, chunks[1]);

        // Events log (from local buffer)
        let max_events = (chunks[2].height.saturating_sub(2)) as usize;
        let recent_events: Vec<Line> = self
            .event_buffer
            .iter()
            .rev()
            .take(max_events)
            .rev()
            .map(|e| {
                // Use event kind for symbol and color, override with level for errors/warnings
                let (symbol, symbol_color) = match e.level {
                    LogLevel::Error => ("✗", theme::RED),
                    LogLevel::Warn => (e.kind.symbol(), theme::YELLOW),
                    _ => (e.kind.symbol(), e.kind.color()),
                };

                // Build spans with highlighted paths, URLs, and timings
                let mut spans = vec![
                    Span::styled(format!("{} ", symbol), Style::default().fg(symbol_color)),
                ];
                spans.extend(highlight_message(&e.message, e.kind));
                Line::from(spans)
            })
            .collect();
        let events_widget = Paragraph::new(recent_events)
            .block(Block::default().title(" 📋 Activity ").borders(Borders::ALL).border_style(Style::default().fg(theme::FG_GUTTER)));
        frame.render_widget(events_widget, chunks[2]);

        // Footer
        let debug_indicator = if self.salsa_debug { "●" } else { "○" };
        let debug_color = if self.salsa_debug {
            theme::GREEN
        } else {
            theme::FG_DARK
        };
        let footer = Paragraph::new(Line::from(vec![
            Span::raw("?").tn_yellow(),
            Span::raw(" help  ").tn_fg_dark(),
            Span::raw("p").tn_yellow(),
            Span::raw(" public  ").tn_fg_dark(),
            Span::raw("d").tn_yellow(),
            Span::raw(" debug ").tn_fg_dark(),
            Span::styled(debug_indicator, Style::default().fg(debug_color)),
            Span::raw("  ").tn_fg_dark(),
            Span::raw("l").tn_yellow(),
            Span::raw(" ").tn_fg_dark(),
            Span::raw(self.log_level.as_str()).tn_cyan(),
            Span::raw("  ").tn_fg_dark(),
            Span::raw("q").tn_yellow(),
            Span::raw(" quit").tn_fg_dark(),
        ]))
        .style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(footer, chunks[3]);

        // Help overlay
        if self.show_help {
            self.draw_help_overlay(frame, area);
        }
    }

    fn draw_help_overlay(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use crate::theme::{self, TokyoNight};
        use ratatui::widgets::Clear;

        // Center the help panel
        let help_width = 40u16;
        let help_height = 14u16;
        let x = area.width.saturating_sub(help_width) / 2;
        let y = area.height.saturating_sub(help_height) / 2;
        let help_area = ratatui::layout::Rect::new(
            x,
            y,
            help_width.min(area.width),
            help_height.min(area.height),
        );

        // Clear the area behind the popup
        frame.render_widget(Clear, help_area);

        let help_text = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  ?").tn_yellow(),
                Span::raw("      Toggle this help").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  p").tn_yellow(),
                Span::raw("      Toggle public/local mode").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  d").tn_yellow(),
                Span::raw("      Toggle salsa debug logs").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  l").tn_yellow(),
                Span::raw("      Cycle log level").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  q").tn_yellow(),
                Span::raw("      Quit / close panel").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  Ctrl+C").tn_yellow(),
                Span::raw(" Force quit").tn_fg(),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("  💻").tn_green(),
                Span::raw(" = localhost only").tn_fg(),
            ]),
            Line::from(vec![
                Span::raw("  🏠").tn_yellow(),
                Span::raw(" = LAN (home network)").tn_fg(),
            ]),
        ];

        let help_widget = Paragraph::new(help_text)
            .block(
                Block::default()
                    .title(" ❓ Help ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::CYAN)),
            )
            .style(Style::default().bg(theme::BG_DARK));

        frame.render_widget(help_widget, help_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_progress_ratio() {
        let mut task = TaskProgress::new("Test");
        assert_eq!(task.ratio(), 0.0);

        task.start(10);
        assert_eq!(task.ratio(), 0.0);

        task.advance();
        task.advance();
        assert!((task.ratio() - 0.2).abs() < f64::EPSILON);

        task.finish();
        assert_eq!(task.ratio(), 1.0);
    }
}
