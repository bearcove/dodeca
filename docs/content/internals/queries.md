+++
title = "Query Reference"
weight = 30
+++

Every computation is a Salsa query. Queries are memoized and track dependencies automatically.

**Inputs** (raw data from disk): `SourceFile`, `TemplateFile`, `SassFile`, `StaticFile`, `OgTemplateFile`.

**Content**: `parse_file` → `build_tree` → `render_page` / `render_section` → `all_rendered_html` → `build_site`.

**Templates**: `load_template` → `load_all_templates` → renders.

**Styles**: `load_sass` → `compile_sass` → `css_output` (cache-busted).

**Images**: `image_metadata` → `image_input_hash` → `process_image` (JXL + WebP at multiple sizes).

**Fonts**: `font_char_analysis` (find used chars) → `subset_font`.

**Assets**: `optimize_svg`, `static_file_output` (cache-busted).
