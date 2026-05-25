use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Document, Element, HtmlElement, KeyboardEvent, MouseEvent};

#[derive(Clone)]
struct SourceTarget {
    element: Element,
    source_file: String,
    line: u32,
}

struct PickerState {
    active: bool,
    mouse_x: f64,
    mouse_y: f64,
    target: Option<SourceTarget>,
    highlight: HtmlElement,
}

pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let Ok(highlight) = document.create_element("div") else {
        return;
    };
    let Ok(highlight) = highlight.dyn_into::<HtmlElement>() else {
        return;
    };

    highlight.set_class_name("dodeca-source-picker-highlight");
    highlight.set_text_content(Some("Open in editor"));
    let style = highlight.style();
    let _ = style.set_property("display", "none");
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("z-index", "99998");
    let _ = style.set_property("pointer-events", "none");
    let _ = style.set_property("box-sizing", "border-box");
    let _ = style.set_property("border", "2px solid #22c55e");
    let _ = style.set_property("background", "rgba(34, 197, 94, 0.08)");
    let _ = style.set_property("color", "#052e16");
    let _ = style.set_property("font", "600 12px system-ui, sans-serif");
    let _ = style.set_property("padding", "4px 6px");
    let _ = style.set_property("border-radius", "4px");
    let _ = style.set_property("box-shadow", "0 0 0 9999px rgba(0, 0, 0, 0.04)");

    let _ = body.append_child(&highlight);

    let state = Rc::new(RefCell::new(PickerState {
        active: false,
        mouse_x: 0.0,
        mouse_y: 0.0,
        target: None,
        highlight,
    }));

    let key_document = document.clone();
    let key_state = state.clone();
    let on_keydown = Closure::<dyn FnMut(KeyboardEvent)>::wrap(Box::new(move |event| {
        if event.code() == "KeyE" && event.alt_key() {
            event.prevent_default();
            let mut state = key_state.borrow_mut();
            state.active = !state.active;
            set_body_cursor(
                &key_document,
                if state.active { "crosshair" } else { "default" },
            );
            update_target(&key_document, &mut state);
        } else if event.code() == "Escape" {
            let mut state = key_state.borrow_mut();
            if state.active {
                event.prevent_default();
                state.active = false;
                state.target = None;
                set_body_cursor(&key_document, "default");
                hide_highlight(&state.highlight);
            }
        }
    }));
    let _ =
        document.add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref());
    on_keydown.forget();

    let move_document = document.clone();
    let move_state = state.clone();
    let on_mousemove = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event| {
        let mut state = move_state.borrow_mut();
        state.mouse_x = event.client_x() as f64;
        state.mouse_y = event.client_y() as f64;
        update_target(&move_document, &mut state);
    }));
    let _ = document
        .add_event_listener_with_callback("mousemove", on_mousemove.as_ref().unchecked_ref());
    on_mousemove.forget();

    let scroll_document = document.clone();
    let scroll_state = state.clone();
    let on_scroll = Closure::<dyn FnMut()>::wrap(Box::new(move || {
        update_target(&scroll_document, &mut scroll_state.borrow_mut());
    }));
    let _ = document.add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref());
    on_scroll.forget();

    let click_document = document.clone();
    let click_state = state.clone();
    let on_click = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event| {
        let target = {
            let mut state = click_state.borrow_mut();
            if !state.active {
                return;
            }
            event.prevent_default();
            event.stop_propagation();
            state.active = false;
            set_body_cursor(&click_document, "default");
            hide_highlight(&state.highlight);
            state.target.take()
        };

        if let Some(target) = target {
            crate::state::open_source(target.source_file, target.line);
        }
    }));
    let _ = document.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    on_click.forget();
}

fn update_target(document: &Document, state: &mut PickerState) {
    if !state.active {
        state.target = None;
        hide_highlight(&state.highlight);
        return;
    }

    state.target = find_source_target(document, state.mouse_x, state.mouse_y);
    match &state.target {
        Some(target) => {
            set_body_cursor(document, "text");
            show_highlight(&state.highlight, &target.element);
        }
        None => {
            set_body_cursor(document, "not-allowed");
            hide_highlight(&state.highlight);
        }
    }
}

fn find_source_target(document: &Document, x: f64, y: f64) -> Option<SourceTarget> {
    let element = document.element_from_point(x as f32, y as f32)?;
    if element
        .closest(".dodeca-devtools, .dodeca-source-picker-highlight")
        .ok()
        .flatten()
        .is_some()
    {
        return None;
    }

    let element = element
        .closest("[data-source-file][data-source-line]")
        .ok()
        .flatten()?;
    let source_file = element.get_attribute("data-source-file")?;
    let line = element
        .get_attribute("data-source-line")?
        .parse::<u32>()
        .ok()
        .filter(|line| *line > 0)?;

    Some(SourceTarget {
        element,
        source_file,
        line,
    })
}

fn show_highlight(highlight: &HtmlElement, element: &Element) {
    let rect = element.get_bounding_client_rect();
    let style = highlight.style();
    let _ = style.set_property("display", "block");
    let _ = style.set_property("top", &format!("{}px", rect.top() - 8.0));
    let _ = style.set_property("left", &format!("{}px", rect.left() - 8.0));
    let _ = style.set_property("width", &format!("{}px", rect.width() + 16.0));
    let _ = style.set_property("min-height", &format!("{}px", rect.height() + 16.0));
}

fn hide_highlight(highlight: &HtmlElement) {
    let _ = highlight.style().set_property("display", "none");
}

fn set_body_cursor(document: &Document, cursor: &str) {
    if let Some(body) = document.body() {
        let _ = body.style().set_property("cursor", cursor);
    }
}
