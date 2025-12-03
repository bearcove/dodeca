#![doc = include_str!("../README.md")]

use std::{collections::HashMap, path::PathBuf};

use chromiumoxide::{Page, browser::Browser};
use clap::Parser;
use color_eyre::eyre::{Context, Result};
use futures::StreamExt;

mod glyph_script;
mod klippa_backend;

#[derive(Parser, Debug)]
#[command(name = "fontcull")]
#[command(about = "Subset fonts based on actual glyph usage from web pages")]
struct Args {
    /// URLs to scan for glyph usage
    #[arg(required = true)]
    urls: Vec<String>,

    /// Font files to subset (glob patterns supported)
    #[arg(long, short = 's')]
    subset: Vec<String>,

    /// Only include glyphs used by these font families (comma-separated)
    #[arg(long, short = 'f')]
    family: Option<String>,

    /// Maximum number of pages to spider (0 = no limit)
    #[arg(long, default_value = "0")]
    spider_limit: usize,

    /// Additional characters to always include (whitelist)
    #[arg(long, short = 'w')]
    whitelist: Option<String>,

    /// Output directory for subset fonts
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

/// Character set per font-family, plus a universal "*" set
#[derive(Debug, Default)]
struct GlyphSets {
    sets: HashMap<String, Vec<u32>>,
}

impl GlyphSets {
    fn new() -> Self {
        Self {
            sets: HashMap::new(),
        }
    }

    fn merge(&mut self, other: HashMap<String, Vec<u32>>) {
        for (family, chars) in other {
            let entry = self.sets.entry(family).or_default();
            for c in chars {
                if !entry.contains(&c) {
                    entry.push(c);
                }
            }
        }
    }

    fn get_for_families(&self, families: Option<&str>) -> Vec<u32> {
        match families {
            Some(filter) => {
                let filter_families: Vec<String> =
                    filter.split(',').map(|s| s.trim().to_lowercase()).collect();

                let mut result = Vec::new();
                for (family, chars) in &self.sets {
                    let family_lower = family.to_lowercase();
                    if filter_families.iter().any(|f| family_lower.contains(f)) {
                        for &c in chars {
                            if !result.contains(&c) {
                                result.push(c);
                            }
                        }
                    }
                }
                result
            }
            None => {
                // Return universal set if present, otherwise union of all
                if let Some(universal) = self.sets.get("*") {
                    universal.clone()
                } else {
                    let mut result = Vec::new();
                    for chars in self.sets.values() {
                        for &c in chars {
                            if !result.contains(&c) {
                                result.push(c);
                            }
                        }
                    }
                    result
                }
            }
        }
    }

    fn add_whitelist(&mut self, whitelist: &str) {
        let entry = self.sets.entry("*".to_string()).or_default();
        for c in whitelist.chars() {
            let code = c as u32;
            if !entry.contains(&code) {
                entry.push(code);
            }
        }
    }
}

/// Convert character codes to Unicode range string (U+XX-YY format)
fn to_unicode_range(mut chars: Vec<u32>) -> String {
    if chars.is_empty() {
        return String::new();
    }

    chars.sort();
    chars.dedup();

    let mut ranges = Vec::new();
    let mut start = chars[0];
    let mut end = chars[0];

    for &c in &chars[1..] {
        if c == end + 1 {
            end = c;
        } else {
            if start == end {
                ranges.push(format!("U+{:X}", start));
            } else {
                ranges.push(format!("U+{:X}-{:X}", start, end));
            }
            start = c;
            end = c;
        }
    }

    // Don't forget the last range
    if start == end {
        ranges.push(format!("U+{:X}", start));
    } else {
        ranges.push(format!("U+{:X}-{:X}", start, end));
    }

    ranges.join(",")
}

async fn extract_glyphs(page: &Page) -> Result<HashMap<String, Vec<u32>>> {
    let script = glyph_script::GLYPH_SCRIPT;

    let result: serde_json::Value = page
        .evaluate(script)
        .await
        .wrap_err("Failed to execute glyph extraction script")?
        .into_value()
        .wrap_err("Failed to get script result")?;

    let mut sets: HashMap<String, Vec<u32>> = HashMap::new();

    if let Some(obj) = result.as_object() {
        for (family, chars) in obj {
            if let Some(arr) = chars.as_array() {
                let codes: Vec<u32> = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect();
                sets.insert(family.clone(), codes);
            }
        }
    }

    Ok(sets)
}

async fn spider_page(page: &Page, limit: usize) -> Result<Vec<String>> {
    let script = r#"
        (() => {
            const links = Array.from(document.querySelectorAll('a[href]'));
            const currentOrigin = window.location.origin;

            // Normalize URL for deduplication
            const normalizeUrl = (url) => {
                try {
                    const parsed = new URL(url);
                    // Remove trailing slash from pathname (except for root)
                    if (parsed.pathname !== '/' && parsed.pathname.endsWith('/')) {
                        parsed.pathname = parsed.pathname.slice(0, -1);
                    }
                    // Remove fragment (hash) to avoid duplicate content crawling
                    parsed.hash = '';
                    // Sort search params for consistent ordering
                    parsed.searchParams.sort();
                    return parsed.toString();
                } catch (e) {
                    return url;
                }
            };

            const normalizedUrls = new Set();
            return links
                .map(a => a.href)
                .filter(href => href.startsWith(currentOrigin))
                .filter(href => {
                    const normalized = normalizeUrl(href);
                    if (normalizedUrls.has(normalized)) {
                        return false;
                    }
                    normalizedUrls.add(normalized);
                    return true;
                })
                .map(href => normalizeUrl(href));
        })()
    "#;

    let result: serde_json::Value = page
        .evaluate(script)
        .await
        .wrap_err("Failed to execute spider script")?
        .into_value()
        .wrap_err("Failed to get spider result")?;

    let mut urls: Vec<String> = result
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if limit > 0 && urls.len() > limit {
        urls.truncate(limit);
    }

    Ok(urls)
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                .from_env_lossy()
                .add_directive("chromiumoxide=off".parse().unwrap()),
        )
        .init();

    let args = Args::parse();
    tracing::info!(?args, "Starting fontcull");

    // Launch browser
    let (mut browser, mut handler) = Browser::launch(
        chromiumoxide::BrowserConfig::builder()
            .build()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to build browser config: {}", e))?,
    )
    .await
    .wrap_err("Failed to launch browser")?;

    // Spawn handler task (errors are expected from chromiumoxide for unhandled CDP messages)
    let handle = tokio::spawn(async move {
        while let Some(_event) = handler.next().await {
            // Silently handle events - errors here are usually just unhandled CDP messages
        }
    });

    let mut glyph_sets = GlyphSets::new();
    let mut visited_urls = std::collections::HashSet::new();
    let mut urls_to_visit: Vec<String> = args.urls.clone();

    // Process all URLs
    while let Some(url) = urls_to_visit.pop() {
        if visited_urls.contains(&url) {
            continue;
        }
        visited_urls.insert(url.clone());

        tracing::info!("Processing URL: {}", url);

        let page = browser
            .new_page(&url)
            .await
            .wrap_err_with(|| format!("Failed to navigate to {}", url))?;

        // Wait for page to load
        page.wait_for_navigation().await.ok();

        // Extract glyphs
        let glyphs = extract_glyphs(&page).await?;
        tracing::info!("Found {} font families with glyphs", glyphs.len());
        glyph_sets.merge(glyphs);

        // Spider for more URLs if requested
        if args.spider_limit > 0 && visited_urls.len() < args.spider_limit {
            let new_urls = spider_page(&page, args.spider_limit - visited_urls.len()).await?;
            for new_url in new_urls {
                if !visited_urls.contains(&new_url) {
                    urls_to_visit.push(new_url);
                }
            }
        }

        page.close().await.ok();
    }

    // Add whitelist characters
    if let Some(ref whitelist) = args.whitelist {
        glyph_sets.add_whitelist(whitelist);
    }

    // Get final character set
    let chars = glyph_sets.get_for_families(args.family.as_deref());
    let unicode_range = to_unicode_range(chars.clone());

    tracing::info!(
        "Total unique characters: {}, Unicode range: {}",
        chars.len(),
        unicode_range
    );

    // Subset fonts if requested
    if !args.subset.is_empty() {
        let mut font_files = Vec::new();
        for pattern in &args.subset {
            for entry in glob::glob(pattern).wrap_err("Invalid glob pattern")? {
                font_files.push(entry.wrap_err("Glob error")?.display().to_string());
            }
        }

        for font_file in font_files {
            tracing::info!("Subsetting font: {}", font_file);

            let output =
                klippa_backend::subset_with_klippa(&font_file, &chars, args.output.as_ref())?;

            tracing::info!("Created: {}", output.display());
        }
    } else {
        // Just print the unicode range
        println!("{}", unicode_range);
    }

    // Cleanup
    browser.close().await.ok();
    handle.abort();

    Ok(())
}
