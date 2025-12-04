//! Main devtools overlay component

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonVariant},
    icon_button::{IconButton, IconButtonSize},
    icons::{IconTriangleAlert, IconSearch, IconX, IconMaximize, IconMinimize},
};

use crate::state::{DevtoolsState, DevtoolsTab, PanelSize};
use super::{ErrorPanel, ScopeExplorer, Repl};

/// The main devtools overlay that floats above the page
#[component]
pub fn DevtoolsOverlay() -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();
    let panel_visible = state.read().panel_visible;
    let panel_size = state.read().panel_size;
    let has_errors = state.read().has_errors();
    let error_count = state.read().error_count();
    let connection_state = state.read().connection_state;

    // Panel height based on size
    let panel_height = match panel_size {
        PanelSize::Normal => "50vh",
        PanelSize::Expanded => "85vh",
    };
    let min_height = match panel_size {
        PanelSize::Normal => "350px",
        PanelSize::Expanded => "500px",
    };

    rsx! {
        // Always show a small badge in the corner to prove devtools is rendering
        if !panel_visible {
            div {
                class: "dodeca-devtools-indicator",
                style: "
                    position: fixed;
                    bottom: 1rem;
                    right: 1rem;
                    z-index: 99999;
                ",
                if has_errors {
                    // Error button
                    Button {
                        variant: ButtonVariant::Danger,
                        onclick: move |_| {
                            state.write().panel_visible = true;
                        },
                        IconTriangleAlert {}
                        if error_count == 1 {
                            " 1 error"
                        } else {
                            " {error_count} errors"
                        }
                    }
                } else {
                    // Status badge (click to open devtools)
                    button {
                        style: "
                            display: flex;
                            align-items: center;
                            gap: 0.25rem;
                            padding: 0.25rem 0.5rem;
                            background: #252525;
                            color: #a3a3a3;
                            border: 1px solid #333;
                            border-radius: 0.375rem;
                            cursor: pointer;
                            font-size: 0.75rem;
                            font-family: system-ui, -apple-system, sans-serif;
                        ",
                        onclick: move |_| {
                            state.write().panel_visible = true;
                        },
                        "🔷"
                        match connection_state {
                            crate::state::ConnectionState::Connected => rsx! {
                                span { style: "color: #22c55e;", "●" }
                            },
                            crate::state::ConnectionState::Connecting => rsx! {
                                span { style: "color: #f59e0b;", "●" }
                            },
                            crate::state::ConnectionState::Disconnected => rsx! {
                                span { style: "color: #ef4444;", "●" }
                            },
                        }
                    }
                }
            }
        }

        // Main panel
        if panel_visible {
            div {
                class: "dodeca-devtools-panel",
                style: "
                    position: fixed;
                    bottom: 0;
                    left: 0;
                    right: 0;
                    height: {panel_height};
                    min-height: {min_height};
                    max-height: 90vh;
                    z-index: 99999;
                    background: #1a1a1a;
                    border-top: 1px solid #333;
                    display: flex;
                    flex-direction: column;
                    font-family: system-ui, -apple-system, sans-serif;
                    color: #e5e5e5;
                    transition: height 0.2s ease;
                ",

                // Header
                div {
                    class: "dodeca-devtools-header",
                    style: "
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        padding: 0.5rem 1rem;
                        border-bottom: 1px solid #333;
                        background: #252525;
                    ",

                    // Logo and title
                    div {
                        style: "display: flex; align-items: center; gap: 0.5rem;",
                        span { style: "font-weight: 600;", "🔷 Dodeca Devtools" }
                        if has_errors {
                            span {
                                style: "
                                    background: #ef4444;
                                    color: white;
                                    padding: 0.125rem 0.5rem;
                                    border-radius: 9999px;
                                    font-size: 0.75rem;
                                ",
                                "{error_count}"
                            }
                        }
                    }

                    // Tabs
                    DevtoolsTabs {}

                    // Control buttons
                    div {
                        style: "display: flex; gap: 0.25rem;",

                        // Expand/collapse button
                        IconButton {
                            size: IconButtonSize::Small,
                            aria_label: if panel_size == PanelSize::Expanded { "Collapse panel".to_string() } else { "Expand panel".to_string() },
                            onclick: move |_| {
                                let mut s = state.write();
                                s.panel_size = match s.panel_size {
                                    PanelSize::Normal => PanelSize::Expanded,
                                    PanelSize::Expanded => PanelSize::Normal,
                                };
                            },
                            if panel_size == PanelSize::Expanded {
                                IconMinimize {}
                            } else {
                                IconMaximize {}
                            }
                        }

                        // Close button
                        IconButton {
                            size: IconButtonSize::Small,
                            aria_label: "Close devtools".to_string(),
                            onclick: move |_| {
                                state.write().panel_visible = false;
                            },
                            IconX {}
                        }
                    }
                }

                // Content area
                div {
                    style: "
                        flex: 1;
                        overflow: auto;
                        padding: 1rem;
                    ",

                    match state.read().active_tab {
                        DevtoolsTab::Errors => rsx! { ErrorPanel {} },
                        DevtoolsTab::Scope => rsx! { ScopeExplorer {} },
                        DevtoolsTab::Repl => rsx! { Repl {} },
                    }
                }
            }
        }
    }
}

#[component]
fn DevtoolsTabs() -> Element {
    let mut state = use_context::<Signal<DevtoolsState>>();
    let active_tab = state.read().active_tab;
    let has_errors = state.read().has_errors();

    rsx! {
        div {
            style: "display: flex; gap: 0.25rem;",

            TabButton {
                active: active_tab == DevtoolsTab::Errors,
                onclick: move |_| state.write().active_tab = DevtoolsTab::Errors,
                IconTriangleAlert {}
                " Errors"
                if has_errors {
                    span {
                        style: "
                            margin-left: 0.25rem;
                            width: 0.5rem;
                            height: 0.5rem;
                            background: #ef4444;
                            border-radius: 50%;
                            display: inline-block;
                        "
                    }
                }
            }

            TabButton {
                active: active_tab == DevtoolsTab::Scope,
                onclick: move |_| state.write().active_tab = DevtoolsTab::Scope,
                IconSearch {}
                " Scope"
            }

            TabButton {
                active: active_tab == DevtoolsTab::Repl,
                onclick: move |_| state.write().active_tab = DevtoolsTab::Repl,
                ">"
                " REPL"
            }
        }
    }
}

#[component]
fn TabButton(
    active: bool,
    onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    let bg = if active { "#333" } else { "transparent" };
    let color = if active { "#fff" } else { "#a3a3a3" };

    rsx! {
        button {
            style: "
                display: flex;
                align-items: center;
                gap: 0.25rem;
                padding: 0.375rem 0.75rem;
                background: {bg};
                color: {color};
                border: none;
                border-radius: 0.375rem;
                cursor: pointer;
                font-size: 0.875rem;
                transition: all 0.15s;
            ",
            onclick: move |evt| onclick.call(evt),
            {children}
        }
    }
}
