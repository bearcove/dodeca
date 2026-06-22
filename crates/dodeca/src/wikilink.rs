//! Context-first `[[wikilink]]` resolution.
//!
//! Resolves a raw wikilink target (the `data-wiki-target` the markdown cell
//! emits) against the site, given the page it appears on. Grammar:
//!
//! ```text
//! link   ::= [ source ":" ] target
//! target ::= bare-slug      // context-first: the page's own section, then each
//!                           //   ancestor section, then the source globally.
//!                           //   Ambiguous only if the *chosen* scope has two.
//!          | path/to/page   // section-relative, else source-root-relative
//!          | ./rel | ../rel // explicit, relative to the page's own directory
//!          | @/abs/path     // source-root-absolute
//! source ::= a source's identity (its name); orthogonal — it may precede any
//!            target form and switches resolution into that source's tree.
//! ```
//!
//! The win over a flat global namespace: a bare `[[core-ml]]` on a page under
//! `tech/` resolves to the sibling `tech/core-ml` even if `bee/impl/core-ml`
//! also exists — the nearer scope wins, with no global ambiguity error.

use std::collections::HashMap;

/// What a target resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Resolved to exactly one route.
    Resolved(String),
    /// The chosen scope held more than one match — the author must qualify.
    Ambiguous(Vec<String>),
    /// No matching page anywhere in scope.
    NotFound,
}

/// The site structure a [`Resolver`] needs. Routes are normalized: a leading
/// slash, no trailing slash, root is `""`.
pub struct Resolver {
    /// Every valid route (sections and pages).
    routes: std::collections::HashSet<String>,
    /// Routes that are sections (have children).
    sections: std::collections::HashSet<String>,
    /// Section route → its direct children as `(leaf slug, route)`.
    children: HashMap<String, Vec<(String, String)>>,
    /// Each source's `(name, mount route)`, longest mount first (so the most
    /// specific mount wins when locating a route's source).
    sources: Vec<(String, String)>,
}

impl Resolver {
    pub fn new(
        routes: impl IntoIterator<Item = String>,
        sections: impl IntoIterator<Item = String>,
        sources: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let routes: std::collections::HashSet<String> =
            routes.into_iter().map(|r| norm(&r)).collect();
        let sections: std::collections::HashSet<String> =
            sections.into_iter().map(|r| norm(&r)).collect();
        // Index each route under its parent section.
        let mut children: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for route in &routes {
            if let Some((parent, slug)) = split_leaf(route) {
                children
                    .entry(parent)
                    .or_default()
                    .push((slug, route.to_string()));
            }
        }
        let mut sources: Vec<(String, String)> = sources
            .into_iter()
            .map(|(name, mount)| (name, norm(&mount)))
            .collect();
        sources.sort_by_key(|(_, mount)| std::cmp::Reverse(mount.len()));
        Self {
            routes,
            sections,
            children,
            sources,
        }
    }

    /// Resolve `target` as authored on the page at `current_route`.
    pub fn resolve(&self, current_route: &str, target: &str) -> Resolution {
        let current = norm(current_route);
        let target = target.trim();

        // Optional `source:` prefix (orthogonal to every target form). Only a
        // known source name counts, so `@/x`, `./x`, and `http://…` aren't
        // mistaken for it. A prefix re-roots resolution into that source's tree,
        // so the *context* becomes the source root (the current page's section
        // in some other source is meaningless).
        let (source_root, rest, context) = match target.split_once(':') {
            Some((name, rest)) if self.source_mount(name).is_some() => {
                let root = self.source_mount(name).unwrap();
                (root.clone(), rest, root)
            }
            _ => (self.source_root_of(&current), target, current.clone()),
        };
        let rest = rest.trim();

        // Directory used for `./`/`../` resolution: the page's own dir, or the
        // source root when re-rooted by a prefix.
        let page_dir = if context == current {
            parent_of(&current)
        } else {
            context.clone()
        };

        if let Some(abs) = rest.strip_prefix("@/") {
            return self.exists_or_not_found(join(&source_root, abs));
        }
        if rest.starts_with("./") || rest.starts_with("../") {
            return match resolve_relative(&page_dir, rest) {
                Some(route) => self.exists_or_not_found(route),
                None => Resolution::NotFound,
            };
        }
        if rest.contains('/') {
            // path/to/page: section-relative first, then source-root-relative.
            let section = self.section_of(&context);
            let section_rel = join(&section, rest);
            if self.routes.contains(&section_rel) {
                return Resolution::Resolved(section_rel);
            }
            return self.exists_or_not_found(join(&source_root, rest));
        }

        // bare-slug: walk scopes nearest-first from the context.
        self.resolve_bare(&context, &source_root, rest)
    }

    /// Context-first bare-slug lookup: the page's own section, then each
    /// ancestor up to the source root, then the source globally. The first scope
    /// that contains the slug decides; two matches *in that scope* is ambiguous.
    fn resolve_bare(&self, current: &str, source_root: &str, slug: &str) -> Resolution {
        let want = slug_key(slug);
        let mut scope = self.section_of(current);
        loop {
            let matches = self.children_matching(&scope, &want);
            if !matches.is_empty() {
                return decide(matches);
            }
            if scope == source_root || scope.is_empty() {
                break;
            }
            scope = self.section_of(&parent_of(&scope));
        }
        // Global within the source: any route under the source root whose leaf
        // slug matches.
        let global: Vec<String> = self
            .routes
            .iter()
            .filter(|r| under(r, source_root))
            .filter(|r| {
                split_leaf(r)
                    .map(|(_, s)| slug_key(&s) == want)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        decide(global)
    }

    fn children_matching(&self, section: &str, want: &str) -> Vec<String> {
        self.children
            .get(section)
            .into_iter()
            .flatten()
            .filter(|(slug, _)| slug_key(slug) == *want)
            .map(|(_, route)| route.clone())
            .collect()
    }

    /// The nearest section at or above `route` (a section is its own section).
    fn section_of(&self, route: &str) -> String {
        let mut current = route.to_string();
        loop {
            if self.sections.contains(&current) {
                return current;
            }
            if current.is_empty() {
                return String::new();
            }
            current = parent_of(&current);
        }
    }

    /// The mount route of the source containing `route` (most specific mount).
    fn source_root_of(&self, route: &str) -> String {
        self.sources
            .iter()
            .find(|(_, mount)| under(route, mount))
            .map(|(_, mount)| mount.clone())
            .unwrap_or_default()
    }

    fn source_mount(&self, name: &str) -> Option<String> {
        self.sources
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, mount)| mount.clone())
    }

    fn exists_or_not_found(&self, route: String) -> Resolution {
        if self.routes.contains(&route) {
            Resolution::Resolved(route)
        } else {
            Resolution::NotFound
        }
    }
}

fn decide(mut matches: Vec<String>) -> Resolution {
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => Resolution::NotFound,
        1 => Resolution::Resolved(matches.into_iter().next().unwrap()),
        _ => Resolution::Ambiguous(matches),
    }
}

/// Normalize a route: leading slash, no trailing slash, root is `""`.
fn norm(route: &str) -> String {
    let trimmed = route.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

/// The parent route (`/a/b` → `/a`, `/a` → `""`).
fn parent_of(route: &str) -> String {
    match route.rfind('/') {
        Some(0) | None => String::new(),
        Some(i) => route[..i].to_string(),
    }
}

/// Split a route into `(parent, leaf-slug)`; `None` for the root.
fn split_leaf(route: &str) -> Option<(String, String)> {
    let i = route.rfind('/')?;
    Some((
        if i == 0 {
            String::new()
        } else {
            route[..i].to_string()
        },
        route[i + 1..].to_string(),
    ))
}

/// Join a base route with a source-relative path.
fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{rel}")
    }
}

/// Resolve a `./` or `../` path against the page's directory, honoring `..`.
fn resolve_relative(page_dir: &str, rel: &str) -> Option<String> {
    let mut segments: Vec<&str> = page_dir.split('/').filter(|s| !s.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            seg => segments.push(seg),
        }
    }
    Some(if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    })
}

/// Whether `route` is at or under `base` (both normalized).
fn under(route: &str, base: &str) -> bool {
    base.is_empty() || route == base || route.starts_with(&format!("{base}/"))
}

/// Case/punctuation-insensitive slug comparison key (`Core-ML` == `core-ml`).
fn slug_key(slug: &str) -> String {
    slug.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> Resolver {
        // A two-source site: the main site, plus a `facet` source mounted at /facet.
        let routes = [
            "/tech",
            "/tech/core-ml",
            "/tech/core-ai",
            "/bee",
            "/bee/impl",
            "/bee/impl/core-ml",
            "/guide",
            "/guide/primer",
            "/reference/spec/scalars",
            "/facet",
            "/facet/tech/core-ml",
        ]
        .map(String::from);
        let sections = [
            "",
            "/tech",
            "/bee",
            "/bee/impl",
            "/guide",
            "/reference",
            "/reference/spec",
            "/facet",
            "/facet/tech",
        ]
        .map(String::from);
        let sources = [
            (String::new(), String::new()),
            ("facet".to_string(), "/facet".to_string()),
        ];
        Resolver::new(routes, sections, sources)
    }

    fn resolve(from: &str, target: &str) -> Resolution {
        resolver().resolve(from, target)
    }

    #[test]
    fn bare_slug_prefers_the_nearest_scope_no_global_ambiguity() {
        // From inside tech/, bare [[core-ml]] is the sibling — even though
        // bee/impl/core-ml also exists. The nearer scope wins.
        assert_eq!(
            resolve("/tech/core-ai", "core-ml"),
            Resolution::Resolved("/tech/core-ml".into())
        );
        // From inside bee/impl/, the same bare slug is *its* sibling.
        assert_eq!(
            resolve("/bee/impl", "core-ml"),
            Resolution::Resolved("/bee/impl/core-ml".into())
        );
    }

    #[test]
    fn bare_slug_walks_up_to_an_ancestor_scope() {
        // From bee/impl, bare [[tech]] isn't a sibling; walking up finds the
        // top-level section globally.
        assert_eq!(
            resolve("/bee/impl/core-ml", "core-ai"),
            Resolution::Resolved("/tech/core-ai".into())
        );
    }

    #[test]
    fn bare_slug_is_case_and_punctuation_insensitive() {
        assert_eq!(
            resolve("/tech/core-ai", "Core ML"),
            Resolution::Resolved("/tech/core-ml".into())
        );
    }

    #[test]
    fn path_target_is_section_relative_then_root_relative() {
        // section-relative: from the /reference section, [[spec/scalars]] is
        // /reference/spec/scalars.
        assert_eq!(
            resolve("/reference", "spec/scalars"),
            Resolution::Resolved("/reference/spec/scalars".into())
        );
        // root-relative fallback: from an unrelated page, the same path still
        // resolves against the source root.
        assert_eq!(
            resolve("/guide/primer", "reference/spec/scalars"),
            Resolution::Resolved("/reference/spec/scalars".into())
        );
    }

    #[test]
    fn relative_targets_resolve_against_the_page_dir() {
        assert_eq!(
            resolve("/tech/core-ai", "./core-ml"),
            Resolution::Resolved("/tech/core-ml".into())
        );
        assert_eq!(
            resolve("/bee/impl/core-ml", "../../tech/core-ml"),
            Resolution::Resolved("/tech/core-ml".into())
        );
    }

    #[test]
    fn root_absolute_target() {
        assert_eq!(
            resolve("/bee/impl", "@/tech/core-ml"),
            Resolution::Resolved("/tech/core-ml".into())
        );
    }

    #[test]
    fn source_prefix_switches_into_that_source_tree() {
        // facet:tech/core-ml resolves within the /facet source, not the main one.
        assert_eq!(
            resolve("/bee/impl", "facet:tech/core-ml"),
            Resolution::Resolved("/facet/tech/core-ml".into())
        );
        // bare slug under a source prefix is context-first within that source.
        assert_eq!(
            resolve("/tech", "facet:core-ml"),
            Resolution::Resolved("/facet/tech/core-ml".into())
        );
    }

    #[test]
    fn missing_target_is_not_found() {
        assert_eq!(resolve("/tech", "nonexistent"), Resolution::NotFound);
        assert_eq!(resolve("/tech", "@/nope"), Resolution::NotFound);
    }
}
