//! Main devtools overlay component

use dioxus::prelude::*;
use glade::components::{
    button::{Button, ButtonVariant},
    icon_button::{IconButton, IconButtonSize},
    icons::{IconTriangleAlert, IconSearch, IconX, IconMaximize, IconMinimize, IconChevronRight},
    tabs::{Tabs, TabList, Tab, TabsVariant},
};

use crate::state::{DevtoolsState, DevtoolsTab, PanelSize};
use super::{ErrorPanel, ScopeExplorer, Repl};

/// Inline dodeca logo SVG
const DODECA_LOGO: &str = r##"<svg viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg" width="18" height="18">
  <path fill="#7aba7a" d="m42.73999 18.59998h-20.42999l-6.40003 19.40002 16.47003 11.98999 16.58002-11.98999z"/>
  <path fill="#5d8a5d" d="m13.95001 37.56001s6.52997-19.79005 6.52997-19.79005c-.91948-1.267-5.1235-7.08827-5.95995-8.25001 0 0-9.99 13.36005-9.99 13.36005-.13.17999-.20001.38995-.20001.60999l.26996 16.59998s8.64801-2.34309 9.35003-2.52996z"/>
  <path fill="#4a6a4a" d="m5.20003 42.00001s9.56999 13.23998 9.56999 13.23998c.13.16998.29999.29999.5.37l15.40997 5.09003c.09111-1.37.55626-7.49243.66002-8.98005.00001.00001-16.80998-12.24999-16.80998-12.24999s-8.84918 2.3889-9.33 2.53004z"/>
  <path fill="#6a9a6a" d="m50.34003 39.45996-17 12.31c-.11033 1.58125-.55533 7.50644-.66004 8.99005 0 0 16.12-4.98004 16.12-4.98004.21002-.06.39001-.20001.52002-.37l9.77002-13.75c-.38045-.09106-8.75-2.20001-8.75-2.20001z"/>
  <path fill="#8fbc8f" d="m22.09998 16.59998h20.84003c.84413-1.28877 4.57061-6.89599 5.33003-8.08996-.00001 0-15.67005-5.45002-15.67005-5.45002-.20996-.08002-.44-.08002-.64996-.01001l-15.79999 5.31c.26188.35504 5.94995 8.23999 5.94995 8.23999z"/>
  <path fill="#6a9a6a" d="m44.58002 17.75 6.33997 19.79999c1.47743.36511 7.3731 1.85349 8.75 2.19001 0 0-.01001-16.22999-.01001-16.22999 0-.21002-.06995-.41003-.19-.58002l-9.57996-13.23999c-.76876 1.19512-4.47377 6.75716-5.31 8.06z"/>
</svg>"##;

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
                            font-family: 'Fira Code', 'SF Mono', Consolas, monospace;
                        ",
                        onclick: move |_| {
                            state.write().panel_visible = true;
                        },
                        span {
                            style: "display: flex; align-items: center;",
                            dangerous_inner_html: DODECA_LOGO,
                        }
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
                    font-family: 'Fira Code', 'SF Mono', Consolas, monospace;
                    color: #e5e5e5;
                    transition: height 0.2s ease;
                ",

                // Resize handle at top
                div {
                    style: "
                        height: 4px;
                        background: #333;
                        cursor: ns-resize;
                        transition: background 0.1s;
                    ",
                    onmouseenter: move |_| {},
                }

                // Header
                div {
                    class: "dodeca-devtools-header",
                    style: "
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        padding: 0.375rem 0.75rem;
                        border-bottom: 1px solid #333;
                        background: #252525;
                    ",

                    // Logo and title
                    div {
                        style: "display: flex; align-items: center; gap: 0.5rem;",
                        span {
                            style: "display: flex; align-items: center;",
                            dangerous_inner_html: DODECA_LOGO,
                        }
                        span { style: "font-weight: 600; font-size: 0.875rem;", "Devtools" }
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
        Tabs {
            variant: TabsVariant::Pills,
            TabList {
                Tab {
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

                Tab {
                    active: active_tab == DevtoolsTab::Scope,
                    onclick: move |_| state.write().active_tab = DevtoolsTab::Scope,
                    IconSearch {}
                    " Scope"
                }

                Tab {
                    active: active_tab == DevtoolsTab::Repl,
                    onclick: move |_| state.write().active_tab = DevtoolsTab::Repl,
                    IconChevronRight {}
                    " REPL"
                }
            }
        }
    }
}
