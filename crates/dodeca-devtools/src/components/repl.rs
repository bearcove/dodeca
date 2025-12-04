//! Expression REPL component for evaluating template expressions

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonSize},
    icons::IconChevronRight,
};

use crate::protocol::ClientMessage;
use crate::state::{send_message, DevtoolsState};

/// Interactive REPL for evaluating template expressions
#[component]
pub fn Repl() -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();
    let mut input = use_signal(|| String::new());
    let mut request_counter = use_signal(|| 0u32);

    let mut on_submit = move |_| {
        let expr = input();
        if expr.trim().is_empty() {
            return;
        }

        // Get snapshot ID from current error (if any)
        let snapshot_id = state
            .read()
            .current_error()
            .map(|e| e.snapshot_id.clone())
            .unwrap_or_default();

        if snapshot_id.is_empty() {
            // TODO: Show error - no snapshot available
            return;
        }

        let id = request_counter();
        request_counter.set(id + 1);

        send_message(&ClientMessage::Eval {
            request_id: id,
            snapshot_id,
            expression: expr.clone(),
        });

        // Add to history
        state.write().repl_history.push(expr);
        input.set(String::new());
    };

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100%;",

            // Header info
            div {
                style: "
                    padding: 0.5rem;
                    background: #252525;
                    border-radius: 0.375rem;
                    margin-bottom: 0.75rem;
                    font-size: 0.875rem;
                    color: #a3a3a3;
                ",
                "> Evaluate template expressions against the current scope"
            }

            // History
            div {
                style: "
                    flex: 1;
                    overflow-y: auto;
                    font-family: 'SF Mono', Consolas, monospace;
                    font-size: 0.875rem;
                    margin-bottom: 0.75rem;
                ",

                for (i, expr) in state.read().repl_history.iter().enumerate() {
                    div {
                        style: "
                            padding: 0.5rem;
                            border-bottom: 1px solid #333;
                        ",

                        // Input
                        div {
                            style: "display: flex; gap: 0.5rem; color: #60a5fa;",
                            span { style: "color: #525252;", "In[{i}]:" }
                            span { "{expr}" }
                        }

                        // TODO: Show output when we have results
                        div {
                            style: "color: #737373; margin-top: 0.25rem; padding-left: 3rem;",
                            "..."
                        }
                    }
                }

                if state.read().repl_history.is_empty() {
                    div {
                        style: "
                            color: #525252;
                            text-align: center;
                            padding: 2rem;
                        ",
                        "No expressions evaluated yet. Try something like:"
                        pre {
                            style: "
                                background: #252525;
                                padding: 0.5rem;
                                border-radius: 0.25rem;
                                margin-top: 0.5rem;
                                color: #a3a3a3;
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
                    padding-top: 0.75rem;
                    border-top: 1px solid #333;
                ",

                div {
                    style: "flex: 1;",
                    input {
                        style: "
                            width: 100%;
                            padding: 0.5rem 0.75rem;
                            background: #0d0d0d;
                            border: 1px solid #333;
                            border-radius: 0.375rem;
                            color: #e5e5e5;
                            font-family: 'SF Mono', Consolas, monospace;
                            font-size: 0.875rem;
                            outline: none;
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
                }

                Button {
                    size: ButtonSize::Small,
                    onclick: move |_| on_submit(()),
                    disabled: state.read().current_error().is_none(),
                    IconChevronRight {}
                    " Run"
                }
            }

            // Status
            if state.read().current_error().is_none() {
                div {
                    style: "
                        margin-top: 0.5rem;
                        padding: 0.5rem;
                        background: rgba(251, 191, 36, 0.1);
                        border: 1px solid rgba(251, 191, 36, 0.3);
                        border-radius: 0.375rem;
                        font-size: 0.75rem;
                        color: #fbbf24;
                    ",
                    "⚠️ No error snapshot available. Trigger a template error to evaluate expressions."
                }
            }
        }
    }
}
