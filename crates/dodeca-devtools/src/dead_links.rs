use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Element, MouseEvent};

use crate::protocol::DeadLinkTarget;

const WIKI_LINK_PREFIX: &str = "dodeca-wiki:";

pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let on_click = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event| {
        let Some(target) = dead_link_target(&event) else {
            return;
        };

        event.prevent_default();
        event.stop_propagation();
        crate::state::open_dead_link(target);
    }));

    let _ = document.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    on_click.forget();
}

fn dead_link_target(event: &MouseEvent) -> Option<DeadLinkTarget> {
    let element = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .or_else(|| {
            web_sys::window()?
                .document()?
                .element_from_point(event.client_x() as f32, event.client_y() as f32)
        })?;
    let anchor = element.closest("a[data-dead]").ok().flatten()?;

    if anchor.closest(".dodeca-devtools").ok().flatten().is_some() {
        return None;
    }

    let href = anchor.get_attribute("href")?;
    if let Some(key) = href.strip_prefix(WIKI_LINK_PREFIX) {
        let title = anchor
            .get_attribute("data-wiki-target")
            .or_else(|| anchor.text_content())
            .unwrap_or_else(|| key.to_string());
        return Some(DeadLinkTarget::Wiki {
            key: key.to_string(),
            title,
        });
    }

    if href.starts_with('/') && !href.starts_with("/__") {
        return Some(DeadLinkTarget::Internal {
            href,
            title: anchor.text_content().unwrap_or_default(),
        });
    }

    None
}
