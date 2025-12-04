//! Scope explorer component for inspecting template variables

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonVariant, ButtonSize},
    icons::{IconChevronRight, IconChevronDown, IconRefreshCw},
};

use crate::protocol::{ClientMessage, ScopeEntry, ScopeValue};
use crate::state::{send_message, DevtoolsState};

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
                    padding: 0.5rem 0.75rem;
                    background: #252525;
                    border-radius: 0.375rem;
                    margin-bottom: 0.5rem;
                ",
                span {
                    style: "font-weight: 500; color: #a3a3a3; font-size: 0.8125rem;",
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
                    font-family: 'Fira Code', 'SF Mono', Consolas, monospace;
                    font-size: 0.8125rem;
                    background: #0d0d0d;
                    border: 1px solid #333;
                    border-radius: 0.375rem;
                ",

                if scope_loading {
                    div {
                        style: "padding: 1.5rem; color: #525252; text-align: center;",
                        "Loading scope..."
                    }
                } else if scope_entries.is_empty() {
                    div {
                        style: "padding: 1.5rem; color: #525252; text-align: center;",
                        "No scope data available. Navigate to a page to see its template scope."
                    }
                } else {
                    div {
                        style: "padding: 0.25rem 0;",
                        for entry in scope_entries {
                            ScopeEntryRow {
                                entry: entry.clone(),
                                path: vec![entry.name.clone()],
                                depth: 0,
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A single entry in the scope tree
#[component]
fn ScopeEntryRow(entry: ScopeEntry, path: Vec<String>, depth: u32) -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();
    let mut expanded = use_signal(|| false);

    let indent = depth * 12;
    let cursor = if entry.expandable { "pointer" } else { "default" };
    let padding_left = indent + 8;

    // Get path key for cache lookup
    let path_key = path.join(".");

    // Check if we have cached children for this path
    let children = state.read().scope_children.get(&path_key).cloned().unwrap_or_default();
    let is_loading = expanded() && children.is_empty() && entry.expandable;

    // Clone for closures
    let path_for_click = path.clone();
    let entry_expandable = entry.expandable;

    rsx! {
        div {
            // Entry row
            div {
                style: "
                    display: flex;
                    align-items: center;
                    padding: 0.1875rem 0.5rem;
                    padding-left: {padding_left}px;
                    cursor: {cursor};
                    background: transparent;
                    transition: background 0.1s;
                    line-height: 1.4;
                ",
                onclick: move |_| {
                    if entry_expandable {
                        let is_expanded = expanded();
                        if !is_expanded {
                            // Check if we need to fetch children
                            let path_key = path_for_click.join(".");
                            let has_children = state.read().scope_children.contains_key(&path_key);

                            if !has_children {
                                // Fetch children
                                let request_id = {
                                    let mut s = state.write();
                                    let id = s.next_request_id;
                                    s.next_request_id += 1;
                                    s.pending_scope_requests.insert(id, path_for_click.clone());
                                    id
                                };

                                send_message(&ClientMessage::GetScope {
                                    request_id,
                                    snapshot_id: None,
                                    path: Some(path_for_click.clone()),
                                });
                            }
                        }
                        expanded.set(!is_expanded);
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
                        if is_loading {
                            span { style: "font-size: 0.6rem;", "..." }
                        } else if expanded() {
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
                    style: "color: #93c5fd;",
                    "{entry.name}"
                }

                span {
                    style: "color: #404040; margin: 0 0.25rem;",
                    ":"
                }

                // Value
                ScopeValueDisplay { value: entry.value.clone() }
            }

            // Children (when expanded)
            if expanded() && !children.is_empty() {
                div {
                    for child in children {
                        ScopeEntryRow {
                            entry: child.clone(),
                            path: {
                                let mut p = path.clone();
                                p.push(child.name.clone());
                                p
                            },
                            depth: depth + 1,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ScopeValueDisplay(value: ScopeValue) -> Element {
    let (color, text) = match &value {
        ScopeValue::Null => ("#525252", "null".to_string()),
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
            style: "color: {color};",
            "{text}"
        }
    }
}
