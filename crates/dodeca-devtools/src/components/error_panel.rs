//! Error panel component for displaying template errors

use dioxus::prelude::*;
use glade::components::{
    badge::Badge,
    button::{Button, ButtonVariant, ButtonSize},
    empty_state::EmptyState,
    icons::{IconCheck, IconChevronRight, IconFileText},
};

use crate::protocol::{ClientMessage, ErrorInfo};
use crate::state::{send_message, DevtoolsState, DevtoolsTab};

/// Panel showing template errors
#[component]
pub fn ErrorPanel() -> Element {
    let state = use_context::<Signal<DevtoolsState>>();
    let errors = &state.read().errors;

    if errors.is_empty() {
        return rsx! {
            EmptyState {
                icon: rsx! { IconCheck {} },
                title: "No errors".to_string(),
                description: "All templates are rendering correctly.".to_string(),
            }
        };
    }

    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 1rem;",

            for error in errors.iter() {
                ErrorCard { error: error.clone() }
            }
        }
    }
}

#[component]
fn ErrorCard(error: ErrorInfo) -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();

    rsx! {
        div {
            style: "
                background: #252525;
                border: 1px solid #ef4444;
                border-radius: 0.5rem;
                overflow: hidden;
            ",

            // Header
            div {
                style: "
                    display: flex;
                    align-items: center;
                    justify-content: space-between;
                    padding: 0.75rem 1rem;
                    background: rgba(239, 68, 68, 0.1);
                    border-bottom: 1px solid #333;
                ",

                div {
                    style: "display: flex; align-items: center; gap: 0.5rem;",
                    span { style: "color: #ef4444; font-size: 1.25rem;", "❌" }
                    span { style: "font-weight: 600;", "Template Error" }
                    Badge { "{error.route}" }
                }

                if let Some(ref template) = error.template {
                    div {
                        style: "display: flex; align-items: center; gap: 0.25rem; color: #a3a3a3; font-size: 0.875rem;",
                        IconFileText {}
                        "{template}"
                        if let Some(line) = error.line {
                            ":{line}"
                        }
                    }
                }
            }

            // Error message
            div {
                style: "padding: 1rem;",

                pre {
                    style: "
                        background: #1a1a1a;
                        border: 1px solid #333;
                        border-radius: 0.375rem;
                        padding: 0.75rem;
                        margin: 0;
                        overflow-x: auto;
                        font-family: 'SF Mono', Consolas, monospace;
                        font-size: 0.875rem;
                        color: #f87171;
                        white-space: pre-wrap;
                        word-wrap: break-word;
                    ",
                    "{error.message}"
                }
            }

            // Source snippet
            if let Some(ref snippet) = error.source_snippet {
                div {
                    style: "padding: 0 1rem 1rem;",

                    div {
                        style: "
                            background: #0d0d0d;
                            border: 1px solid #333;
                            border-radius: 0.375rem;
                            overflow: hidden;
                        ",

                        // Source lines
                        for line in snippet.lines.iter() {
                            SourceLine {
                                number: line.number,
                                content: line.content.clone(),
                                is_error: line.number == snippet.error_line,
                            }
                        }
                    }
                }
            }

            // Available variables hint
            if !error.available_variables.is_empty() {
                div {
                    style: "
                        padding: 0.75rem 1rem;
                        background: #1a1a1a;
                        border-top: 1px solid #333;
                        font-size: 0.875rem;
                    ",

                    div {
                        style: "color: #a3a3a3; margin-bottom: 0.5rem;",
                        "Available variables:"
                    }

                    div {
                        style: "display: flex; flex-wrap: wrap; gap: 0.25rem;",
                        for var in error.available_variables.iter() {
                            span {
                                style: "
                                    background: #333;
                                    padding: 0.125rem 0.5rem;
                                    border-radius: 0.25rem;
                                    font-family: 'SF Mono', Consolas, monospace;
                                    font-size: 0.75rem;
                                ",
                                "{var}"
                            }
                        }
                    }
                }
            }

            // Actions
            div {
                style: "
                    display: flex;
                    gap: 0.5rem;
                    padding: 0.75rem 1rem;
                    background: #1a1a1a;
                    border-top: 1px solid #333;
                ",

                Button {
                    size: ButtonSize::Small,
                    onclick: {
                        let snapshot_id = error.snapshot_id.clone();
                        move |_| {
                            // Switch to scope tab and load scope for this error
                            state.write().active_tab = DevtoolsTab::Scope;
                            send_message(&ClientMessage::GetScope {
                                request_id: 1,
                                snapshot_id: Some(snapshot_id.clone()),
                                path: None,
                            });
                        }
                    },
                    IconChevronRight {}
                    " Explore Scope"
                }

                Button {
                    size: ButtonSize::Small,
                    variant: ButtonVariant::Ghost,
                    onclick: {
                        let route = error.route.clone();
                        move |_| {
                            send_message(&ClientMessage::DismissError { route: route.clone() });
                            state.write().errors.retain(|e| e.route != route);
                        }
                    },
                    "Dismiss"
                }
            }
        }
    }
}

#[component]
fn SourceLine(number: u32, content: String, is_error: bool) -> Element {
    let bg = if is_error { "rgba(239, 68, 68, 0.15)" } else { "transparent" };
    let border = if is_error { "2px solid #ef4444" } else { "2px solid transparent" };

    rsx! {
        div {
            style: "
                display: flex;
                background: {bg};
                border-left: {border};
            ",

            // Line number
            span {
                style: "
                    width: 3rem;
                    padding: 0.25rem 0.5rem;
                    text-align: right;
                    color: #525252;
                    background: #0a0a0a;
                    user-select: none;
                    font-family: 'SF Mono', Consolas, monospace;
                    font-size: 0.75rem;
                ",
                "{number}"
            }

            // Code
            {
                let color = if is_error { "#fca5a5" } else { "#d4d4d4" };
                rsx! {
                    pre {
                        style: "
                            flex: 1;
                            margin: 0;
                            padding: 0.25rem 0.75rem;
                            font-family: 'SF Mono', Consolas, monospace;
                            font-size: 0.875rem;
                            color: {color};
                        ",
                        "{content}"
                    }
                }
            }
        }
    }
}
