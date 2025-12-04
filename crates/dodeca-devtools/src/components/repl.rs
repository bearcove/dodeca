//! Expression REPL component for evaluating template expressions

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonSize},
    icons::IconChevronRight,
};

use crate::protocol::{ClientMessage, ScopeValue};
use crate::state::{send_message, DevtoolsState, ReplEntry};

/// Interactive REPL for evaluating template expressions
#[component]
pub fn Repl() -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();
    let mut input = use_signal(|| String::new());

    let mut on_submit = move |_| {
        let expr = input();
        if expr.trim().is_empty() {
            return;
        }

        // Get next request ID from state
        let id = {
            let mut s = state.write();
            let id = s.next_request_id;
            s.next_request_id += 1;

            // Add entry to history with pending result
            s.repl_history.push(ReplEntry {
                expression: expr.clone(),
                result: None,
            });

            // Track pending evaluation
            s.pending_evals.insert(id, expr.clone());
            id
        };

        // Send eval request (snapshot_id not used, server uses current route)
        send_message(&ClientMessage::Eval {
            request_id: id,
            snapshot_id: String::new(),
            expression: expr,
        });

        input.set(String::new());
    };

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100%; font-family: 'Fira Code', 'SF Mono', Consolas, monospace;",

            // Header info
            div {
                style: "
                    padding: 0.5rem 0.75rem;
                    background: #252525;
                    border-radius: 0.375rem;
                    margin-bottom: 0.5rem;
                    font-size: 0.8125rem;
                    color: #737373;
                ",
                "Evaluate template expressions against the current page's scope"
            }

            // History
            div {
                style: "
                    flex: 1;
                    overflow-y: auto;
                    font-size: 0.8125rem;
                    margin-bottom: 0.5rem;
                    background: #0d0d0d;
                    border: 1px solid #333;
                    border-radius: 0.375rem;
                ",

                for (i, entry) in state.read().repl_history.iter().enumerate() {
                    ReplEntryRow { index: i, entry: entry.clone() }
                }

                if state.read().repl_history.is_empty() {
                    div {
                        style: "
                            color: #525252;
                            text-align: center;
                            padding: 1.5rem;
                        ",
                        "No expressions evaluated yet. Try something like:"
                        pre {
                            style: "
                                background: #1a1a1a;
                                padding: 0.75rem;
                                border-radius: 0.25rem;
                                margin-top: 0.75rem;
                                color: #737373;
                                text-align: left;
                                display: inline-block;
                            ",
                            "page.title\n"
                            "section.pages | length\n"
                            "config.base_url"
                        }
                    }
                }
            }

            // Input area
            div {
                style: "
                    display: flex;
                    gap: 0.5rem;
                    align-items: stretch;
                ",

                input {
                    style: "
                        flex: 1;
                        padding: 0.5rem 0.75rem;
                        background: #0d0d0d;
                        border: 1px solid #333;
                        border-radius: 0.375rem;
                        color: #e5e5e5;
                        font-family: 'Fira Code', 'SF Mono', Consolas, monospace;
                        font-size: 0.8125rem;
                        outline: none;
                        height: 36px;
                        box-sizing: border-box;
                    ",
                    r#type: "text",
                    placeholder: "Enter expression...",
                    value: "{input}",
                    oninput: move |evt| input.set(evt.value().clone()),
                    onkeydown: move |evt| {
                        if evt.key() == Key::Enter {
                            on_submit(());
                        }
                    },
                }

                Button {
                    size: ButtonSize::Small,
                    onclick: move |_| on_submit(()),
                    IconChevronRight {}
                    " Run"
                }
            }
        }
    }
}

/// Display a single REPL entry
#[component]
fn ReplEntryRow(index: usize, entry: ReplEntry) -> Element {
    rsx! {
        div {
            style: "
                padding: 0.5rem 0.75rem;
                border-bottom: 1px solid #1a1a1a;
            ",

            // Input expression
            div {
                style: "display: flex; gap: 0.5rem; align-items: baseline;",
                span { style: "color: #404040; font-size: 0.75rem;", "In[{index}]" }
                span { style: "color: #60a5fa;", "{entry.expression}" }
            }

            // Output result
            div {
                style: "margin-top: 0.25rem; padding-left: 2.5rem;",
                match &entry.result {
                    None => rsx! {
                        span { style: "color: #525252;", "..." }
                    },
                    Some(Ok(value)) => rsx! {
                        ReplValueDisplay { value: value.clone() }
                    },
                    Some(Err(error)) => rsx! {
                        // Error is already HTML (converted from ANSI on server)
                        pre {
                            style: "margin: 0; white-space: pre-wrap; color: #ccc; font-size: 0.8125rem;",
                            dangerous_inner_html: "{error}",
                        }
                    },
                }
            }
        }
    }
}

/// Display a scope value in the REPL
#[component]
fn ReplValueDisplay(value: ScopeValue) -> Element {
    let (color, text) = match &value {
        ScopeValue::Null => ("#525252", "null".to_string()),
        ScopeValue::Bool(b) => ("#c084fc", b.to_string()),
        ScopeValue::Number(n) => ("#4ade80", n.to_string()),
        ScopeValue::String(s) => {
            let display = if s.len() > 100 {
                format!("\"{}...\"", &s[..97])
            } else {
                format!("\"{}\"", s)
            };
            ("#fbbf24", display)
        }
        ScopeValue::Array { length, preview } => ("#60a5fa", format!("Array({}) {}", length, preview)),
        ScopeValue::Object { fields, preview } => ("#f472b6", format!("Object({}) {}", fields, preview)),
    };

    rsx! {
        span { style: "color: {color};", "{text}" }
    }
}
