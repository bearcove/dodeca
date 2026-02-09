+++
title = "Query Reference"
weight = 30
+++

Every computation is a Picante query. Queries are memoized and track dependencies automatically.

## Inputs

Raw data from disk — these are `#[picante::input]` types that enter the system when files change:

`SourceFile`, `TemplateFile`, `SassFile`, `StaticFile`, `OgTemplateFile`.

## Content Pipeline

```mermaid
flowchart LR
    SF[SourceFile] --> parse_file
    parse_file --> build_tree
    build_tree --> render_page
    build_tree --> render_section
    render_page --> all_rendered_html
    render_section --> all_rendered_html
    all_rendered_html --> build_site
```

## Template Pipeline

```mermaid
flowchart LR
    TF[TemplateFile] --> load_template
    load_template --> load_all_templates
    load_all_templates --> render_page
    load_all_templates --> render_section
```

## Style Pipeline

```mermaid
flowchart LR
    SASS[SassFile] --> load_sass --> compile_sass --> css_output["css_output<br/>(cache-busted)"]
```

## Image Pipeline

```mermaid
flowchart LR
    IMG["StaticFile<br/>image"] --> image_metadata --> image_input_hash --> process_image
    process_image --> JXL[JXL variants]
    process_image --> WebP[WebP variants]
    process_image --> JPEG[JPEG fallback]
```

## Font Pipeline

```mermaid
flowchart LR
    HTML[all_rendered_html] --> font_char_analysis["font_char_analysis<br/>(find used chars)"]
    FONT["StaticFile<br/>font"] --> font_char_analysis
    font_char_analysis --> subset_font
```

## Asset Pipeline

```mermaid
flowchart LR
    SVG["StaticFile<br/>SVG"] --> optimize_svg --> static_file_output["static_file_output<br/>(cache-busted)"]
    OTHER["StaticFile<br/>other"] --> static_file_output
```
