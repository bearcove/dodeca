//! Per-source templates: each mounted content source renders with its OWN
//! chrome, and a shared bare template name (`shell.html`) resolves within the
//! rendering source — never the other's.
//!
//! The `multi-source-site` fixture is an aggregator with two `local` sources:
//! `kb` mounted at `/` and `wiki` mounted at `/wiki`. Both keep a `page.html`
//! that `{% extends "shell.html" %}`, but each source's `shell.html` is
//! distinct (`KB-SHELL` vs `WIKI-SHELL`). If template resolution were global,
//! the wiki page would inherit the kb's `shell.html` (the bare-key collision);
//! per-source filtering is what keeps them apart.

use super::*;

/// A page served from the primary (`/`) source renders with the kb's own
/// `page.html` and its `{% extends "shell.html" %}` resolves to the kb's shell.
pub async fn primary_source_uses_its_own_chrome() {
    let site = TestSite::new("multi-source-site");
    let html = site.get("/hello/").await;
    html.assert_ok();
    html.assert_contains("KB-SHELL");
    html.assert_contains(r#"data-chrome="kb""#);
    html.assert_contains(r#"data-page="kb""#);
    html.assert_contains("This is a KB page body.");
    // The wiki's chrome must never leak into a primary-source render.
    html.assert_not_contains("WIKI-SHELL");
}

/// A page served from the `/wiki` mount renders with the wiki's own `page.html`
/// and — critically — its `{% extends "shell.html" %}` resolves to the wiki's
/// `shell.html`, not the kb's same-named template.
pub async fn mounted_source_uses_its_own_chrome() {
    let site = TestSite::new("multi-source-site");
    let html = site.get("/wiki/note/").await;
    html.assert_ok();
    html.assert_contains("WIKI-SHELL");
    html.assert_contains(r#"data-chrome="wiki""#);
    html.assert_contains(r#"data-page="wiki""#);
    html.assert_contains("This is a wiki page body.");
    // The kb's chrome must never leak into a mounted-source render.
    html.assert_not_contains("KB-SHELL");
}

/// A mounted source authored its internal links root-absolute (`/other/`, `/`)
/// as if it lived at the site root. Mounting relocates it under `/wiki`, so
/// those links must be rewritten to the mount-prefixed routes — otherwise they
/// would point at the primary site. This rides the same path_map rewrite the
/// rest of dodeca uses (cache-busting, asset aliasing), scoped to the source's
/// own routes.
pub async fn mounted_source_links_are_localized() {
    let site = TestSite::new("multi-source-site");
    let html = site.get("/wiki/note/").await;
    html.assert_ok();
    // /other/ is a real wiki route → localized to /wiki/other/.
    html.assert_contains(r#"href="/wiki/other/""#);
    html.assert_not_contains(r#"href="/other/""#);
    // The root-absolute home link resolves to the wiki home, not the kb root.
    html.assert_contains(r#"href="/wiki/""#);
    // An anchored link keeps its #fragment while the path is localized.
    html.assert_contains(r##"href="/wiki/other/#details""##);
}

/// A mounted source whose repo can't be cloned at startup (here: a bogus git
/// path) must be skipped without taking the site down — the primary keeps
/// serving and the broken mount 404s. This guards mounting a private repo
/// before the deploy bot has read access: it degrades, it does not crash.
pub async fn unclonable_mounted_source_does_not_break_the_site() {
    let site = TestSite::new("mount-clone-fail-site");
    // The primary source serves normally.
    let home = site.get("/").await;
    home.assert_ok();
    home.assert_contains("MAIN-OK");
    // The mount whose clone failed simply has no routes → 404, site stays up.
    let broken = site.get("/broken/").await;
    assert_eq!(
        broken.status, 404,
        "a skipped mount should 404, got {}",
        broken.status
    );
}

/// The section templates are per-source too: the root section (`/`) uses the
/// kb's `index.html`; the wiki's root section (`/wiki/`) uses the wiki's
/// `section.html` (its route isn't `/`, so it's a section, not an index).
pub async fn section_templates_are_per_source() {
    let site = TestSite::new("multi-source-site");

    let root = site.get("/").await;
    root.assert_ok();
    root.assert_contains("KB-SHELL");
    root.assert_contains("KB-HOME");
    root.assert_not_contains("WIKI-SHELL");

    let wiki_home = site.get("/wiki/").await;
    wiki_home.assert_ok();
    wiki_home.assert_contains("WIKI-SHELL");
    wiki_home.assert_contains("WIKI-SECTION");
    wiki_home.assert_not_contains("KB-SHELL");
}
