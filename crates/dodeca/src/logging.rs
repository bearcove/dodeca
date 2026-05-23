//! Tracing/logging infrastructure for dodeca
//!
//! Provides:
//! - TUI layer that routes log events to the Activity panel
//! - Dynamic filtering with picante debug toggle
//! - Per-crate filtering with RUST_LOG-style expressions
//! - Slow query logging (spans >50ms)
//! - Standard env filter for non-TUI mode

use crate::tui::{EventKind, LogEvent, LogLevel};
use std::sync::Arc;
use std::sync::RwLock;
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

/// Default tracing filter - INFO for everything, with noisy deps silenced.
/// This is used by:
/// - `ddc build` (non-TUI mode)
/// - `ddc serve --no-tui` (non-TUI mode)
/// - `ddc serve` (TUI mode default)
/// - Cells (pushed via TracingConfig RPC)
pub const DEFAULT_TRACING_FILTER: &str = "info,hyper=warn,h2=warn,tower=warn,rustls=warn";

/// A tracing layer that sends formatted events to a channel (for TUI Activity panel)
pub struct TuiLayer {
    tx: Sender<LogEvent>,
    picante_debug: Arc<AtomicBool>,
    log_level: Arc<AtomicU8>,
    custom_filter: Arc<RwLock<Option<LogFilter>>>,
}

impl TuiLayer {
    pub fn new(tx: Sender<LogEvent>) -> Self {
        Self {
            tx,
            picante_debug: Arc::new(AtomicBool::new(false)),
            log_level: Arc::new(AtomicU8::new(TuiLogLevel::Info as u8)),
            custom_filter: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a handle to update the filter dynamically
    pub fn filter_handle(&self) -> FilterHandle {
        FilterHandle {
            picante_debug: self.picante_debug.clone(),
            log_level: self.log_level.clone(),
            custom_filter: self.custom_filter.clone(),
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

    /// Check if an event should be shown based on custom filter
    fn should_show_with_filter(&self, target: &str, level: Level) -> bool {
        let filter = self.custom_filter.read().unwrap();
        if let Some(ref f) = *filter {
            f.should_show(target, level)
        } else {
            // Fall back to simple level filter
            self.get_log_level().should_show(level)
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
                        fields: vec![],
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
        // Always filter cranelift_jit to error level (very noisy)
        } else if target.starts_with("cranelift_jit") {
            if level > Level::ERROR {
                return;
            }
        } else {
            // Filter based on custom filter or simple level
            if !self.should_show_with_filter(target, level) {
                return;
            }
        }

        // Extract message and fields
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let (msg, fields) = if let Some(parts) = visitor.into_parts() {
            parts
        } else {
            (format!("{}: {}", target, metadata.name()), vec![])
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
            fields,
        });
    }
}

/// Log level for TUI filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiLogLevel {
    /// Show nothing
    Off,
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
            TuiLogLevel::Off => TuiLogLevel::Error,
            TuiLogLevel::Error => TuiLogLevel::Warn,
            TuiLogLevel::Warn => TuiLogLevel::Info,
            TuiLogLevel::Info => TuiLogLevel::Debug,
            TuiLogLevel::Debug => TuiLogLevel::Trace,
            TuiLogLevel::Trace => TuiLogLevel::Off,
        }
    }

    /// Short display name for TUI
    pub fn as_str(&self) -> &'static str {
        match self {
            TuiLogLevel::Off => "OFF",
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
            TuiLogLevel::Off => false,
            TuiLogLevel::Error => level == Level::ERROR,
            TuiLogLevel::Warn => level <= Level::WARN,
            TuiLogLevel::Info => level <= Level::INFO,
            TuiLogLevel::Debug => level <= Level::DEBUG,
            TuiLogLevel::Trace => true,
        }
    }
}

/// A parsed log filter with per-target level overrides
#[derive(Clone, Debug)]
pub struct LogFilter {
    /// Default level for targets not explicitly configured
    default_level: TuiLogLevel,
    /// Per-target level overrides (target prefix -> level)
    /// More specific targets should match first
    targets: Vec<(String, TuiLogLevel)>,
    /// Original filter expression for display
    expression: String,
}

impl LogFilter {
    /// Parse a RUST_LOG-style filter expression
    /// Examples: "info", "warn,dodeca=debug", "dodeca::cells=trace,hyper=off"
    pub fn parse(expr: &str) -> Option<Self> {
        let expr = expr.trim();
        if expr.is_empty() {
            return None;
        }

        let mut default_level = TuiLogLevel::Info;
        let mut targets = Vec::new();

        for part in expr.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some((target, level_str)) = part.split_once('=') {
                let level = parse_level(level_str)?;
                targets.push((target.to_string(), level));
            } else {
                // No '=' means it's a default level
                default_level = parse_level(part)?;
            }
        }

        // Sort targets by length (descending) so more specific matches come first
        targets.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Some(LogFilter {
            default_level,
            targets,
            expression: expr.to_string(),
        })
    }

    /// Check if an event with the given target and level should be shown
    pub fn should_show(&self, target: &str, level: Level) -> bool {
        // Find the most specific matching target
        for (prefix, filter_level) in &self.targets {
            if target.starts_with(prefix) {
                return filter_level.should_show(level);
            }
        }
        // Fall back to default
        self.default_level.should_show(level)
    }

    /// Get the original expression for display
    pub fn expression(&self) -> &str {
        &self.expression
    }
}

/// Parse a level string (info, debug, warn, error, trace, off)
fn parse_level(s: &str) -> Option<TuiLogLevel> {
    match s.to_lowercase().as_str() {
        "error" | "err" => Some(TuiLogLevel::Error),
        "warn" | "warning" => Some(TuiLogLevel::Warn),
        "info" => Some(TuiLogLevel::Info),
        "debug" | "dbg" => Some(TuiLogLevel::Debug),
        "trace" => Some(TuiLogLevel::Trace),
        "off" | "none" => Some(TuiLogLevel::Off),
        _ => None,
    }
}

/// Handle for dynamically updating the log filter
#[derive(Clone)]
pub struct FilterHandle {
    picante_debug: Arc<AtomicBool>,
    log_level: Arc<AtomicU8>,
    custom_filter: Arc<RwLock<Option<LogFilter>>>,
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
    /// This also clears any custom filter expression
    pub fn cycle_log_level(&self) -> TuiLogLevel {
        // Clear custom filter when cycling
        *self.custom_filter.write().unwrap() = None;

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
            5 => TuiLogLevel::Off,
            _ => TuiLogLevel::Info, // fallback
        }
    }

    /// Set a custom filter expression (RUST_LOG style)
    /// Returns the parsed filter expression for display, or None if invalid
    pub fn set_filter(&self, expr: &str) -> Option<String> {
        if let Some(filter) = LogFilter::parse(expr) {
            let display = filter.expression().to_string();
            *self.custom_filter.write().unwrap() = Some(filter);
            Some(display)
        } else if expr.trim().is_empty() {
            // Empty expression clears the filter
            *self.custom_filter.write().unwrap() = None;
            Some(String::new())
        } else {
            None
        }
    }

    /// Get the current filter expression for display (if any)
    #[allow(dead_code)]
    pub fn get_filter_expression(&self) -> Option<String> {
        self.custom_filter
            .read()
            .unwrap()
            .as_ref()
            .map(|f| f.expression().to_string())
    }
}

/// Visitor to extract the message and all fields from an event
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl MessageVisitor {
    /// Extract message and fields separately
    fn into_parts(self) -> Option<(String, Vec<(String, String)>)> {
        Some((self.message?, self.fields))
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
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
    } else if msg_lower.contains("search") {
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

/// Initialize tracing for TUI mode (uses the centralized DEFAULT_TRACING_FILTER)
/// Returns a FilterHandle for dynamic filter updates
/// Starts with picante debug disabled - use 'd' key to toggle
pub fn init_tui_tracing(event_tx: Sender<LogEvent>) -> FilterHandle {
    let tui_layer = TuiLayer::new(event_tx);
    let handle = tui_layer.filter_handle();

    // Set the default filter
    handle.set_filter(DEFAULT_TRACING_FILTER);

    tracing_subscriber::registry().with(tui_layer).init();

    handle
}

/// Initialize tracing for non-TUI mode (uses RUST_LOG env var or DEFAULT_TRACING_FILTER)
pub fn init_standard_tracing() {
    // Start with user's RUST_LOG or our default filter that silences dependencies
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_TRACING_FILTER))
        .add_directive("cranelift_jit=error".parse().unwrap());
    let log_format = std::env::var("DDC_LOG_FORMAT")
        .unwrap_or_default()
        .to_lowercase();
    let log_time = std::env::var("DDC_LOG_TIME")
        .unwrap_or_default()
        .to_lowercase();

    if log_format == "json" {
        // JSON format - structured logging
        let json_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_target(true)
            .with_current_span(false)
            .with_span_list(false);

        match log_time.as_str() {
            "utc" => {
                tracing_subscriber::registry()
                    .with(
                        json_layer
                            .with_timer(tracing_subscriber::fmt::time::SystemTime)
                            .with_filter(filter),
                    )
                    .init();
            }
            "none" => {
                tracing_subscriber::registry()
                    .with(json_layer.without_time().with_filter(filter))
                    .init();
            }
            _ => {
                tracing_subscriber::registry()
                    .with(
                        json_layer
                            .with_timer(tracing_subscriber::fmt::time::uptime())
                            .with_filter(filter),
                    )
                    .init();
            }
        }
    } else {
        // Standard format - human readable
        let fmt_layer = tracing_subscriber::fmt::layer().with_target(true).compact();
        match log_time.as_str() {
            "utc" => {
                tracing_subscriber::registry()
                    .with(
                        fmt_layer
                            .with_timer(tracing_subscriber::fmt::time::SystemTime)
                            .with_filter(filter),
                    )
                    .init();
            }
            "none" => {
                tracing_subscriber::registry()
                    .with(fmt_layer.without_time().with_filter(filter))
                    .init();
            }
            _ => {
                tracing_subscriber::registry()
                    .with(
                        fmt_layer
                            .with_timer(tracing_subscriber::fmt::time::uptime())
                            .with_filter(filter),
                    )
                    .init();
            }
        }
    }
}
