+++
title = "Configuration"
weight = 10
+++

dodeca is configured via `.config/dodeca.styx`, using [Styx](https://github.com/bearcove/styx) syntax.

Styx uses `key value` pairs — no colons, no equals signs, no YAML.

The config has three sections:

- **`source {}`** — *composable*, source-scoped settings: what a content
  collection **is** and how to render / validate / execute it. When a source is
  mounted into another site, this travels with it.
- **`site {}`** — *non-composable*, whole-site settings: properties of the
  assembled, published site. Exactly one applies to a build — never composed
  from a mounted source.
- **`mounts (...)`** — *aggregator only*: extra content sources merged into one
  site, each under a URL `path`, composing that source's own `source {}`.

A leaf project sets `source` + `site`. An aggregator adds `mounts`. At least one
of `source` / `mounts` must be present.

## Minimal configuration

```styx
source {
    content content
}

site {
    output public
}
```

## Full reference

### `source {}` — composable, source-scoped

```styx
source {
    content content      # Content directory (relative to the source's root)
    repo "https://github.com/you/proj/tree/main"  # "view source" base URL

    # Code whose source files are scanned for `r[verb rule.id]` coverage refs.
    impls (
        {name rust, include (rust/**/src/**/*.rs), test_include (rust/**/tests/**/*.rs)}
    )

    # Domains to skip when link-checking this source's external links. Unioned
    # into the assembled site's link check.
    skip_domains (
        example.com
    )

    # Custom commands callable from templates via `{{ build("step_name") }}`.
    build_steps {
        git_hash {
            command (git rev-parse --short HEAD)
        }
        version {
            command (cat VERSION)
        }
    }

    # First-class frontmatter schemas keyed by page type.
    page-types {
        Decision @object{
            type @string
        }
    }
}
```

### `site {}` — non-composable, whole-site

```styx
site {
    output public          # Build output directory
    base_url "https://example.com"

    syntax_highlight {
        light_theme github-light
        dark_theme tokyo-night
    }

    code_execution {
        dependencies (
            {name serde, version "1.0"}
            {name tokio, version "1", features (full)}
        )
    }

    # Files that keep their original paths (no cache-busting).
    stable_assets (
        favicon.ico
        favicon.svg
        robots.txt
        CNAME
    )

    link_check {
        mode warn
        rate_limit_ms 200
        skip_domains (
            example.com
        )
    }
}
```

### `mounts (...)` — aggregator

Each entry has a `name` and a URL `path` (which may **not** be `/` — the root is
the top-level `source`), plus a location: either `local` (a content directory on
disk) or `checkout` (a repo directory, optionally with a `content` subpath and a
`git` URL to clone). A mounted source composes its own `source {}`, read from its
`.config/dodeca.styx`.

```styx
source {
    content content
}

mounts (
    {
        name guide
        path /guide
        local ../guide/content
    }
    {
        name api
        path /api
        checkout ../api-repo
        content docs/content
        git "https://github.com/you/api-repo.git"
    }
)

site {
    output public
}
```

## Example: dodeca's own config

This is the configuration dodeca uses for its own documentation site:

```styx
source {
    content docs/content

    build_steps {
        git_hash {
            command (git rev-parse --short HEAD)
        }
    }
}

site {
    output docs/public

    code_execution {
        dependencies (
            {name serde, version "1.0"}
        )
    }

    syntax_highlight {
        light_theme github-light
        dark_theme tokyo-night
    }
}
```
