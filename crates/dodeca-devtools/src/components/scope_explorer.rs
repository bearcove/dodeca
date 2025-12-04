//! Scope explorer component for inspecting template variables

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonVariant, ButtonSize},
    icons::{IconChevronRight, IconChevronDown, IconRefreshCw},
};

use crate::protocol::{ScopeEntry, ScopeValue};
use crate::state::DevtoolsState;

/// Tree view for exploring template scope variables
#[component]
pub fn ScopeExplorer() -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();

    // Request scope when component mounts
    use_effect(move || {
        state.write().request_scope();
    });

    let scope_entries = state.read().scope_entries.clone();
    let scope_loading = state.read().scope_loading;

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100%;",

            // Header with refresh button
            div {
                style: "
                    display: flex;
                    align-items: center;
                    justify-content: space-between;
                    padding: 0.5rem;
                    border-bottom: 1px solid #333;
                    background: #252525;
                ",
                span {
                    style: "font-weight: 600; color: #e5e5e5;",
                    "Template Scope"
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Small,
                    disabled: scope_loading,
                    onclick: move |_| {
                        state.write().request_scope();
                    },
                    IconRefreshCw {}
                    " Refresh"
                }
            }

            // Scope entries
            div {
                style: "
                    flex: 1;
                    overflow: auto;
                    font-family: 'SF Mono', Consolas, monospace;
                    font-size: 0.875rem;
                ",

                if scope_loading {
                    div {
                        style: "padding: 1rem; color: #737373; text-align: center;",
                        "Loading scope..."
                    }
                } else if scope_entries.is_empty() {
                    div {
                        style: "padding: 1rem; color: #737373; text-align: center;",
                        "No scope data available. Navigate to a page to see its template scope."
                    }
                } else {
                    for entry in scope_entries {
                        ScopeEntryRow { entry: entry.clone(), depth: 0 }
                    }
                }
            }
        }
    }
}

/// A single entry in the scope tree
#[component]
fn ScopeEntryRow(entry: ScopeEntry, depth: u32) -> Element {
    let mut expanded = use_signal(|| false);
    let indent = depth * 16;
    let cursor = if entry.expandable { "pointer" } else { "default" };
    let padding_left = indent + 8;

    rsx! {
        div {
            // Entry row
            div {
                style: "
                    display: flex;
                    align-items: center;
                    padding: 0.25rem 0.5rem;
                    padding-left: {padding_left}px;
                    cursor: {cursor};
                    background: transparent;
                    transition: background 0.1s;
                ",
                onmouseenter: move |_| {},
                onclick: move |_| {
                    if entry.expandable {
                        expanded.set(!expanded());
                    }
                },

                // Expand/collapse chevron
                if entry.expandable {
                    span {
                        style: "
                            width: 1rem;
                            height: 1rem;
                            display: flex;
                            align-items: center;
                            justify-content: center;
                            color: #737373;
                        ",
                        if expanded() {
                            IconChevronDown {}
                        } else {
                            IconChevronRight {}
                        }
                    }
                } else {
                    span { style: "width: 1rem;" }
                }

                // Name
                span {
                    style: "
                        color: #93c5fd;
                        font-family: 'SF Mono', Consolas, monospace;
                        font-size: 0.875rem;
                    ",
                    "{entry.name}"
                }

                span {
                    style: "color: #525252; margin: 0 0.25rem;",
                    ":"
                }

                // Value
                ScopeValueDisplay { value: entry.value.clone() }
            }

            // Children (when expanded)
            // TODO: Load children from server
        }
    }
}

#[component]
fn ScopeValueDisplay(value: ScopeValue) -> Element {
    let (color, text) = match &value {
        ScopeValue::Null => ("#737373", "null".to_string()),
        ScopeValue::Bool(b) => ("#c084fc", b.to_string()),
        ScopeValue::Number(n) => ("#4ade80", n.to_string()),
        ScopeValue::String(s) => {
            let display = if s.len() > 50 {
                format!("\"{}...\"", &s[..47])
            } else {
                format!("\"{}\"", s)
            };
            ("#fbbf24", display)
        }
        ScopeValue::Array { length, preview } => ("#60a5fa", format!("Array({}) {}", length, preview)),
        ScopeValue::Object { fields, preview } => ("#f472b6", format!("Object({}) {}", fields, preview)),
    };

    rsx! {
        span {
            style: "
                color: {color};
                font-family: 'SF Mono', Consolas, monospace;
                font-size: 0.875rem;
            ",
            "{text}"
        }
    }
}
