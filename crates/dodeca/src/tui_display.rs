//! Local TUI display loop.
//!
//! This is the former `cell-tui` display implementation wired directly into
//! the monolith. Updates arrive through local channels, and key commands are
//! sent straight to [`crate::host::Host`].

use std::collections::VecDeque;
use std::io::stdout;
use std::time::Duration;

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use tokio::sync::mpsc;

use cell_tui_proto::{
    BindMode, BuildProgress, EventKind, LogEvent, ServerCommand, ServerStatus, TaskStatus,
    TuiDisplay,
};

mod theme {
    use ratatui::style::Color;

    pub const BG_DARK: Color = Color::Rgb(0x16, 0x16, 0x1e);
    pub const FG: Color = Color::Rgb(0xa9, 0xb1, 0xd6);
    pub const FG_DARK: Color = Color::Rgb(0x56, 0x5f, 0x89);
    pub const FG_GUTTER: Color = Color::Rgb(0x3b, 0x40, 0x61);

    pub const BLUE: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
    pub const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
    pub const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
    pub const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
    pub const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
    pub const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
    pub const PURPLE: Color = Color::Rgb(0x9d, 0x7c, 0xd8);

    pub fn http_status_color(status: u16) -> Color {
        match status {
            500..=599 => RED,
            400..=499 => YELLOW,
            300..=399 => CYAN,
            200..=299 => GREEN,
            100..=199 => BLUE,
            _ => FG_DARK,
        }
    }
}

const MAX_EVENTS: usize = 100;

const FILTER_PRESETS: &[&str] = &[
    "info,hyper=warn,h2=warn,tower=warn,rustls=warn",
    "debug,hyper=warn,h2=warn,tower=warn,rustls=warn",
    "trace,hyper=warn,h2=warn,tower=warn,rustls=warn",
    "warn",
];

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}

struct TuiApp {
    progress: BuildProgress,
    server_status: ServerStatus,
    event_buffer: VecDeque<LogEvent>,
    command_tx: mpsc::UnboundedSender<ServerCommand>,
    show_help: bool,
    should_quit: bool,
    filter_preset_index: usize,
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
        if let Some(ref mut input) = self.filter_input {
            match code {
                KeyCode::Enter => {
                    let filter = std::mem::take(input);
                    self.filter_input = None;
                    let _ = self.command_tx.send(ServerCommand::SetLogFilter { filter });
                }
                KeyCode::Esc => {
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
                self.should_quit = true;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('o') => {
                if let Some(url) = self.server_status.urls.first()
                    && let Err(error) = open::that(url)
                {
                    tracing::warn!(%error, %url, "failed to open browser from TUI");
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
                self.filter_preset_index = (self.filter_preset_index + 1) % FILTER_PRESETS.len();
                let filter = FILTER_PRESETS[self.filter_preset_index];
                let _ = self.command_tx.send(ServerCommand::SetLogFilter {
                    filter: filter.to_string(),
                });
            }
            KeyCode::Char('F') => {
                self.filter_input = Some(String::new());
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame) {
        use theme::*;

        let area = frame.area();
        let url_height = self.server_status.urls.len().max(1) as u16 + 2;
        let chunks = Layout::vertical([
            Constraint::Length(url_height),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

        let (network_icon, icon_color) = match self.server_status.bind_mode {
            BindMode::Local => ("local", GREEN),
            BindMode::Lan => ("lan", YELLOW),
        };
        let status = if self.server_status.is_running {
            ("*", GREEN)
        } else {
            ("-", YELLOW)
        };

        let url_lines: Vec<Line> = self
            .server_status
            .urls
            .iter()
            .map(|url| {
                Line::from(vec![
                    Span::raw("  -> ").fg(CYAN),
                    Span::raw(url.clone()).fg(BLUE),
                ])
            })
            .collect();

        let server_title = Line::from(vec![
            Span::raw(" Server "),
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

        let tasks = [
            (&self.progress.parse, "parse"),
            (&self.progress.render, "render"),
            (&self.progress.sass, "sass"),
        ];
        let mut status_spans: Vec<Span> = Vec::new();
        for (i, (task, label)) in tasks.iter().enumerate() {
            if i > 0 {
                status_spans.push(Span::raw("  ").fg(FG_DARK));
            }
            let (color, symbol) = match task.status {
                TaskStatus::Pending => (FG_DARK, "-"),
                TaskStatus::Running => (CYAN, "~"),
                TaskStatus::Done => (GREEN, "+"),
                TaskStatus::Error => (RED, "!"),
            };
            status_spans.push(Span::raw(format!("{label} ")).fg(FG));
            status_spans.push(Span::styled(symbol, Style::default().fg(color)));
        }

        let has_cache = self.server_status.picante_cache_size > 0
            || self.server_status.cas_cache_size > 0
            || self.server_status.code_exec_cache_size > 0;
        if has_cache {
            status_spans.push(Span::raw("  cache ").fg(FG_DARK));
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
                .title(" Status ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG_GUTTER)),
        );
        frame.render_widget(progress_widget, chunks[1]);

        let available_width = chunks[2].width.saturating_sub(2) as usize;
        let max_lines = chunks[2].height.saturating_sub(2) as usize;

        let mut all_lines: Vec<Line> = Vec::new();
        for event in self.event_buffer.iter() {
            all_lines.extend(format_event(event, available_width));
        }

        let skip = all_lines.len().saturating_sub(max_lines);
        let recent_lines: Vec<Line> = all_lines.into_iter().skip(skip).collect();

        let events_widget = Paragraph::new(recent_lines).block(
            Block::default()
                .title(" Activity ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FG_GUTTER)),
        );
        frame.render_widget(events_widget, chunks[2]);

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

        if self.filter_input.is_some() {
            self.draw_filter_input(frame, area);
        }

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
                Span::raw(input.to_string()).fg(CYAN),
                Span::raw("|").fg(YELLOW),
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
                    .title(" Log Filter (RUST_LOG syntax) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(CYAN)),
            )
            .style(Style::default().bg(BG_DARK));

        frame.render_widget(input_widget, input_area);
    }

    fn draw_help_overlay(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use theme::*;

        let help_width = 42u16;
        let help_height = 13u16;
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
                Span::raw("      Open first URL").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  p").fg(YELLOW),
                Span::raw("      Toggle public/local").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  d").fg(YELLOW),
                Span::raw("      Toggle picante debug").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  l").fg(YELLOW),
                Span::raw("      Cycle log level").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  f").fg(YELLOW),
                Span::raw("      Cycle filter presets").fg(FG),
            ]),
            Line::from(vec![
                Span::raw("  F").fg(YELLOW),
                Span::raw("      Enter custom filter").fg(FG),
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
                Span::raw("  local").fg(GREEN),
                Span::raw(" = localhost only").fg(FG),
            ]),
        ];

        let help_widget = Paragraph::new(help_text)
            .block(
                Block::default()
                    .title(" Help ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(CYAN)),
            )
            .style(Style::default().bg(BG_DARK));

        frame.render_widget(help_widget, help_area);
    }
}

fn format_event(event: &LogEvent, max_width: usize) -> Vec<Line<'static>> {
    use theme::*;

    let msg_color = event_color(&event.kind);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if event.fields.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            event.message.clone(),
            Style::default().fg(msg_color),
        )]));
        return lines;
    }

    let mut first_line_spans = vec![Span::styled(
        event.message.clone(),
        Style::default().fg(msg_color),
    )];

    let mut current_line_len = event.message.chars().count();
    let indent = "    ";
    let indent_len = indent.chars().count();
    let mut continuation_spans: Vec<Span<'static>> = Vec::new();
    let mut continuation_len = indent_len;

    for (key, value) in &event.fields {
        let field_len = key.chars().count() + 1 + value.chars().count();
        let total_field_len = 1 + field_len;

        if continuation_spans.is_empty() {
            if current_line_len + total_field_len <= max_width {
                first_line_spans.push(Span::raw(" "));
                first_line_spans.push(Span::styled(key.clone(), Style::default().fg(CYAN)));
                first_line_spans.push(Span::styled("=", Style::default().fg(FG_DARK)));
                first_line_spans.push(Span::styled(value.clone(), Style::default().fg(GREEN)));
                current_line_len += total_field_len;
            } else {
                lines.push(Line::from(std::mem::take(&mut first_line_spans)));
                continuation_spans.push(Span::raw(indent.to_string()));
                continuation_spans.push(Span::styled(key.clone(), Style::default().fg(CYAN)));
                continuation_spans.push(Span::styled("=", Style::default().fg(FG_DARK)));
                continuation_spans.push(Span::styled(value.clone(), Style::default().fg(GREEN)));
                continuation_len = indent_len + field_len;
            }
        } else if continuation_len + total_field_len <= max_width {
            continuation_spans.push(Span::raw(" "));
            continuation_spans.push(Span::styled(key.clone(), Style::default().fg(CYAN)));
            continuation_spans.push(Span::styled("=", Style::default().fg(FG_DARK)));
            continuation_spans.push(Span::styled(value.clone(), Style::default().fg(GREEN)));
            continuation_len += total_field_len;
        } else {
            lines.push(Line::from(std::mem::take(&mut continuation_spans)));
            continuation_spans.push(Span::raw(indent.to_string()));
            continuation_spans.push(Span::styled(key.clone(), Style::default().fg(CYAN)));
            continuation_spans.push(Span::styled("=", Style::default().fg(FG_DARK)));
            continuation_spans.push(Span::styled(value.clone(), Style::default().fg(GREEN)));
            continuation_len = indent_len + field_len;
        }
    }

    if !first_line_spans.is_empty() {
        lines.push(Line::from(first_line_spans));
    }
    if !continuation_spans.is_empty() {
        lines.push(Line::from(continuation_spans));
    }

    lines
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

struct TerminalSession {
    terminal: Option<DefaultTerminal>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        Ok(Self {
            terminal: Some(ratatui::init()),
        })
    }

    fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        self.terminal
            .as_mut()
            .expect("terminal should be active until restored")
    }

    fn restore(&mut self) -> Result<()> {
        if self.terminal.take().is_some() {
            disable_raw_mode()?;
            stdout().execute(LeaveAlternateScreen)?;
            ratatui::restore();
        }
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.terminal.take().is_some() {
            let _ = disable_raw_mode();
            let _ = stdout().execute(LeaveAlternateScreen);
            ratatui::restore();
        }
    }
}

#[derive(Clone)]
pub struct TuiDisplayClient {
    progress_tx: mpsc::UnboundedSender<BuildProgress>,
    event_tx: mpsc::UnboundedSender<LogEvent>,
    status_tx: mpsc::UnboundedSender<ServerStatus>,
}

impl TuiDisplayClient {
    pub async fn update_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send(progress);
    }

    pub async fn push_event(&self, event: LogEvent) {
        let _ = self.event_tx.send(event);
    }

    pub async fn update_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send(status);
    }
}

impl TuiDisplay for TuiDisplayClient {
    async fn update_progress(&self, progress: BuildProgress) {
        self.update_progress(progress).await;
    }

    async fn push_event(&self, event: LogEvent) {
        self.push_event(event).await;
    }

    async fn update_status(&self, status: ServerStatus) {
        self.update_status(status).await;
    }
}

pub fn spawn_tui_display() -> TuiDisplayClient {
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (status_tx, status_rx) = mpsc::unbounded_channel();

    tokio::spawn(run_tui_loop(progress_rx, event_rx, status_rx));

    TuiDisplayClient {
        progress_tx,
        event_tx,
        status_tx,
    }
}

async fn run_tui_loop(
    mut progress_rx: mpsc::UnboundedReceiver<BuildProgress>,
    mut events_rx: mpsc::UnboundedReceiver<LogEvent>,
    mut status_rx: mpsc::UnboundedReceiver<ServerStatus>,
) {
    if let Err(error) = run_tui_loop_inner(&mut progress_rx, &mut events_rx, &mut status_rx).await {
        tracing::error!(%error, "TUI failed");
    }
}

async fn run_tui_loop_inner(
    progress_rx: &mut mpsc::UnboundedReceiver<BuildProgress>,
    events_rx: &mut mpsc::UnboundedReceiver<LogEvent>,
    status_rx: &mut mpsc::UnboundedReceiver<ServerStatus>,
) -> Result<()> {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<ServerCommand>();
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();

    let mut terminal_session = TerminalSession::enter()?;
    let mut app = TuiApp::new(command_tx);

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
                    Err(error) => {
                        tracing::debug!(%error, "TUI key reader stopped");
                        break;
                    }
                },
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(%error, "TUI key poll failed");
                    break;
                }
            }
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = tick.tick() => {}

            Some(key) = key_rx.recv() => {
                app.handle_key(key.code, key.modifiers);
            }

            Some(cmd) = command_rx.recv() => {
                let _ = crate::host::Host::get().handle_tui_command(cmd);
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

        terminal_session
            .terminal_mut()
            .draw(|frame| app.draw(frame))?;

        if app.should_quit {
            while let Ok(cmd) = command_rx.try_recv() {
                let _ = crate::host::Host::get().handle_tui_command(cmd);
            }
            terminal_session.restore()?;
            crate::host::Host::get().signal_exit();
            break;
        }
    }

    Ok(())
}
