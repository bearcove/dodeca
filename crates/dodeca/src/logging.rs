//! Tracing/logging infrastructure for dodeca
//!
//! Provides:
//! - TUI layer that routes log events to the Activity panel
//! - Dynamic filtering with picante debug toggle
//! - Slow query logging (spans >50ms)
//! - Standard env filter for non-TUI mode

use crate::tui::{EventKind, LogEvent, LogLevel};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    Layer,
    filter::EnvFilter,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
};

/// Threshold for logging slow spans (in milliseconds)
const SLOW_SPAN_THRESHOLD_MS: u128 = 50;

/// A tracing layer that sends formatted events to a channel (for TUI Activity panel)
pub struct TuiLayer {
    tx: Sender<LogEvent>,
    picante_debug: Arc<AtomicBool>,
    log_level: Arc<AtomicU8>,
}

impl TuiLayer {
    pub fn new(tx: Sender<LogEvent>) -> Self {
        Self {
            tx,
            picante_debug: Arc::new(AtomicBool::new(false)),
            log_level: Arc::new(AtomicU8::new(TuiLogLevel::Info as u8)),
        }
    }

    /// Get a handle to update the filter dynamically
    pub fn filter_handle(&self) -> FilterHandle {
        FilterHandle {
            picante_debug: self.picante_debug.clone(),
            log_level: self.log_level.clone(),
        }
    }

    /// Get the current log level
    fn get_log_level(&self) -> TuiLogLevel {
        match self.log_level.load(Ordering::Relaxed) {
            0 => TuiLogLevel::Error,
            1 => TuiLogLevel::Warn,
            2 => TuiLogLevel::Info,
            3 => TuiLogLevel::Debug,
            4 => TuiLogLevel::Trace,
            _ => TuiLogLevel::Info,
        }
    }
}

/// Extension data stored with each span for timing
struct SpanTiming {
    start: Instant,
}

impl<S> Layer<S> for TuiLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanTiming {
                start: Instant::now(),
            });
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let extensions = span.extensions();
            if let Some(timing) = extensions.get::<SpanTiming>() {
                let elapsed_ms = timing.start.elapsed().as_millis();
                if elapsed_ms >= SLOW_SPAN_THRESHOLD_MS {
                    let name = span.name();
                    let _ = self.tx.send(LogEvent {
                        level: LogLevel::Warn,
                        kind: EventKind::Build,
                        message: format!("slow query: {} took {}ms", name, elapsed_ms),
                    });
                }
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = *metadata.level();
        let target = metadata.target();

        // Filter picante events (they use INFO and DEBUG levels)
        if target.starts_with("picante") {
            if !self.picante_debug.load(Ordering::Relaxed) {
                return;
            }
        } else {
            // Filter based on current log level
            let current_level = self.get_log_level();
            if !current_level.should_show(level) {
                return;
            }
        }

        // Format the event
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let msg = if let Some(message) = visitor.message {
            message
        } else {
            format!("{}: {}", target, metadata.name())
        };

        // Convert tracing Level to our LogLevel
        let log_level = match level {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => LogLevel::Trace,
        };

        // Detect event kind from message patterns
        let kind = detect_event_kind(&msg, target);

        let _ = self.tx.send(LogEvent {
            level: log_level,
            kind,
            message: msg,
        });
    }
}

/// Log level for TUI filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiLogLevel {
    /// Show only errors
    Error,
    /// Show warnings and errors
    Warn,
    /// Show info, warnings, and errors (default)
    #[default]
    Info,
    /// Show debug and above
    Debug,
    /// Show everything
    Trace,
}

impl TuiLogLevel {
    /// Cycle to the next log level (more verbose)
    pub fn next(self) -> Self {
        match self {
            TuiLogLevel::Error => TuiLogLevel::Warn,
            TuiLogLevel::Warn => TuiLogLevel::Info,
            TuiLogLevel::Info => TuiLogLevel::Debug,
            TuiLogLevel::Debug => TuiLogLevel::Trace,
            TuiLogLevel::Trace => TuiLogLevel::Error,
        }
    }

    /// Short display name for TUI
    pub fn as_str(&self) -> &'static str {
        match self {
            TuiLogLevel::Error => "ERROR",
            TuiLogLevel::Warn => "WARN",
            TuiLogLevel::Info => "INFO",
            TuiLogLevel::Debug => "DEBUG",
            TuiLogLevel::Trace => "TRACE",
        }
    }

    /// Check if a given tracing level should be shown at this TUI level
    pub fn should_show(&self, level: Level) -> bool {
        match self {
            TuiLogLevel::Error => level == Level::ERROR,
            TuiLogLevel::Warn => level <= Level::WARN,
            TuiLogLevel::Info => level <= Level::INFO,
            TuiLogLevel::Debug => level <= Level::DEBUG,
            TuiLogLevel::Trace => true,
        }
    }
}

/// Handle for dynamically updating the log filter
#[derive(Clone)]
pub struct FilterHandle {
    picante_debug: Arc<AtomicBool>,
    log_level: Arc<AtomicU8>,
}

impl FilterHandle {
    /// Toggle picante debug logging, returns new state
    pub fn toggle_picante_debug(&self) -> bool {
        // Toggle and return the new value
        !self.picante_debug.fetch_xor(true, Ordering::Relaxed)
    }

    /// Check if picante debug is currently enabled
    fn _is_picante_debug_enabled(&self) -> bool {
        self.picante_debug.load(Ordering::Relaxed)
    }

    /// Cycle to the next log level (more verbose, wrapping around)
    pub fn cycle_log_level(&self) -> TuiLogLevel {
        let current = self.get_log_level();
        let next = current.next();
        self.log_level.store(next as u8, Ordering::Relaxed);
        next
    }

    /// Get the current log level
    pub fn get_log_level(&self) -> TuiLogLevel {
        match self.log_level.load(Ordering::Relaxed) {
            0 => TuiLogLevel::Error,
            1 => TuiLogLevel::Warn,
            2 => TuiLogLevel::Info,
            3 => TuiLogLevel::Debug,
            4 => TuiLogLevel::Trace,
            _ => TuiLogLevel::Info, // fallback
        }
    }
}

/// Visitor to extract the message field from an event
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

/// Detect event kind from message content and target
fn detect_event_kind(msg: &str, target: &str) -> EventKind {
    // HTTP requests: "GET /path -> 200 in 1.2ms"
    if let Some(status) = parse_http_status(msg) {
        return EventKind::Http { status };
    }

    // Picante/build events
    if target.starts_with("picante") {
        return EventKind::Build;
    }

    // Match on message content patterns
    let msg_lower = msg.to_lowercase();

    if msg_lower.contains("reload") && !msg_lower.contains("live reload") {
        EventKind::Reload
    } else if msg_lower.contains("patch") {
        EventKind::Patch
    } else if msg_lower.contains("search") || msg_lower.contains("pagefind") {
        EventKind::Search
    } else if msg_lower.contains("changed:")
        || msg_lower.contains("modified")
        || msg_lower.contains("watching")
    {
        EventKind::FileChange
    } else if msg_lower.contains("server")
        || msg_lower.contains("listening")
        || msg_lower.contains("binding")
        || msg_lower.contains("browser")
        || msg_lower.contains("connected")
        || msg_lower.contains("disconnected")
    {
        EventKind::Server
    } else if msg_lower.contains("compil")
        || msg_lower.contains("build")
        || msg_lower.contains("render")
        || msg_lower.contains("slow query")
        || msg_lower.contains("loaded")
        || msg_lower.contains("cache")
    {
        EventKind::Build
    } else {
        EventKind::Generic
    }
}

/// Try to parse HTTP status code from log message like "GET /path -> 200 in 1.2ms"
fn parse_http_status(msg: &str) -> Option<u16> {
    let msg = msg.trim();

    // New format: "GET /path -> 200 in 1.2ms"
    if let Some(arrow_pos) = msg.find(" -> ") {
        let after_arrow = &msg[arrow_pos + 4..];
        // Extract status code (first token after arrow)
        let status_end = after_arrow.find(' ').unwrap_or(after_arrow.len());
        let status_str = &after_arrow[..status_end];
        if let Ok(status) = status_str.parse::<u16>() {
            if (100..600).contains(&status) {
                return Some(status);
            }
        }
    }

    // Old format fallback: "200 /path 1.2ms" (first token is status)
    let first_space = msg.find(' ')?;
    let status_str = &msg[..first_space];
    let status: u16 = status_str.parse().ok()?;
    if (100..600).contains(&status) {
        Some(status)
    } else {
        None
    }
}

/// Initialize tracing for TUI mode
/// Returns a FilterHandle for dynamic filter updates
/// Starts with picante debug disabled - use 'd' key to toggle
pub fn init_tui_tracing(event_tx: Sender<LogEvent>) -> FilterHandle {
    let tui_layer = TuiLayer::new(event_tx);
    let handle = tui_layer.filter_handle();

    tracing_subscriber::registry().with(tui_layer).init();

    handle
}

/// Initialize tracing for non-TUI mode (uses RUST_LOG env var)
pub fn init_standard_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_timer(tracing_subscriber::fmt::time::uptime())
                .compact()
                .with_filter(filter),
        )
        .init();
}
