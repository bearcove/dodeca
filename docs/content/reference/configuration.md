+++
title = "Configuration"
weight = 10
+++

dodeca is configured via `.config/dodeca.styx`, using [Styx](https://github.com/bearcove/styx) syntax.

Styx uses `key value` pairs — no colons, no equals signs, no YAML.

## Minimal configuration

```styx
content content
output public
```

## Full reference

### Paths

```styx
content content        # Content directory (relative to project root)
output public          # Build output directory
```

### Syntax highlighting

```styx
syntax_highlight {
    light_theme github-light
    dark_theme tokyo-night
}
```

### Code execution

```styx
code_execution {
    dependencies (
        {name serde, version "1.0"}
        {name tokio, version "1", features (full)}
    )
}
```

### Stable assets

Files that should keep their original paths (no cache-busting):

```styx
stable_assets (
    favicon.ico
    favicon.svg
    robots.txt
    CNAME
)
```

### Link checking

```styx
link_check true
```

### Build steps

Custom commands that can be called from templates via `{{ build("step_name") }}`:

```styx
build_steps {
    git_hash {
        command (git rev-parse --short HEAD)
    }
    version {
        command (cat VERSION)
    }
}
```

## Example: dodeca's own config

This is the configuration dodeca uses for its own documentation site:

```styx
content docs/content
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

build_steps {
    git_hash {
        command (git rev-parse --short HEAD)
    }
}
```
