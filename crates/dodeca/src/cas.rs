//! Content-addressed storage for incremental builds
//!
//! Uses rapidhash for fast hashing and simple file-based persistence.
//! Tracks which files have been written and their content hashes to avoid
//! unnecessary disk writes.
//!
//! Blobs (processed images, decompressed fonts) are stored on disk rather than
//! in the database to keep things lean and fast.

/// CAS version - bump this when making incompatible changes
pub const CAS_VERSION: u32 = 5;

use crate::db::ProcessedImages;
use camino::Utf8Path;
use rapidhash::fast::RapidHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// Global blob storage directory
static BLOB_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Persisted hash map: path -> content hash
#[derive(Default, facet::Facet)]
struct HashStore {
    hashes: HashMap<String, u64>,
}

/// Content-addressed storage for build outputs
pub struct ContentStore {
    path: PathBuf,
    store: HashStore,
}

impl ContentStore {
    /// Open or create a content store at the given path
    pub fn open(path: &Utf8Path) -> eyre::Result<Self> {
        let path = path.as_std_path().to_path_buf();
        let store = if path.exists() {
            if path.is_dir() {
                // Migration: old canopydb created a directory, new format is a single file
                fs::remove_dir_all(&path)?;
                HashStore::default()
            } else {
                let data = fs::read(&path)?;
                facet_postcard::from_slice(&data).unwrap_or_default()
            }
        } else {
            HashStore::default()
        };
        Ok(Self { path, store })
    }

    /// Save the store to disk
    pub fn save(&self) -> eyre::Result<()> {
        let data = facet_postcard::to_vec(&self.store)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, data)?;
        Ok(())
    }

    /// Compute the rapidhash of content
    fn hash(content: &[u8]) -> u64 {
        let mut hasher = RapidHasher::default();
        hasher.write(content);
        hasher.finish()
    }

    /// Write content to a file if it has changed since last build.
    /// Returns true if the file was written, false if skipped (unchanged).
    pub fn write_if_changed(
        &mut self,
        path: &Utf8Path,
        content: &[u8],
    ) -> eyre::Result<bool> {
        let hash = Self::hash(content);
        let path_key = path.as_str().to_string();

        // Check if hash matches
        if self.store.hashes.get(&path_key) == Some(&hash) {
            return Ok(false);
        }

        // Hash differs or not stored - write the file
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;

        // Update stored hash
        self.store.hashes.insert(path_key, hash);

        Ok(true)
    }
}

// ============================================================================
// Asset Cache (global, for processed images, OG images, etc.)
// ============================================================================

/// Ensure .cache is in the nearest .gitignore, or create one if needed
fn ensure_gitignore_has_cache(cache_dir: &Path) {
    // The cache directory name we want to ignore
    let cache_name = ".cache";

    // Start from cache_dir's parent and search upward for .git (repo root) or .gitignore
    let mut search_dir = cache_dir.parent();
    let mut found_gitignore: Option<PathBuf> = None;
    let mut found_git_repo = false;

    while let Some(dir) = search_dir {
        // Check if this is a git repo root
        if dir.join(".git").exists() {
            found_git_repo = true;
            let gitignore_path = dir.join(".gitignore");
            found_gitignore = Some(gitignore_path);
            break;
        }
        // Also check for existing .gitignore (might be in a subdirectory of a repo)
        let gitignore_path = dir.join(".gitignore");
        if gitignore_path.exists() {
            found_gitignore = Some(gitignore_path);
            found_git_repo = true; // Assume we're in a git repo if .gitignore exists
            break;
        }
        search_dir = dir.parent();
    }

    // Only modify .gitignore if we're in a git repository
    if !found_git_repo {
        return;
    }

    let gitignore_path = found_gitignore.unwrap_or_else(|| {
        // No .gitignore found but we're in a git repo, create one as sibling to .cache
        cache_dir.parent().unwrap_or(cache_dir).join(".gitignore")
    });

    // Check if .cache is already in the gitignore
    let needs_update = if gitignore_path.exists() {
        match fs::read_to_string(&gitignore_path) {
            Ok(content) => !content.lines().any(|line| {
                let trimmed = line.trim();
                trimmed == cache_name || trimmed == format!("{}/", cache_name)
            }),
            Err(_) => true, // Can't read, try to append anyway
        }
    } else {
        true // File doesn't exist, need to create it
    };

    if needs_update {
        // Append .cache to gitignore
        let entry = if gitignore_path.exists() {
            // Check if file ends with newline
            let content = fs::read_to_string(&gitignore_path).unwrap_or_default();
            if content.ends_with('\n') || content.is_empty() {
                format!("{}\n", cache_name)
            } else {
                format!("\n{}\n", cache_name)
            }
        } else {
            format!("{}\n", cache_name)
        };

        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)
        {
            use std::io::Write;
            let _ = file.write_all(entry.as_bytes());
            tracing::info!("Added {} to {:?}", cache_name, gitignore_path);
        }
    }
}

/// Initialize the global asset cache (blob storage for images, fonts)
pub fn init_asset_cache(cache_dir: &Path) -> eyre::Result<()> {
    tracing::debug!(cache_dir = %cache_dir.display(), "init_asset_cache called");

    // Ensure .cache is gitignored before creating directories
    ensure_gitignore_has_cache(cache_dir);

    // Blob storage directory (for processed images, decompressed fonts)
    let blob_path = cache_dir.join("blobs");

    // Check version file - if missing or mismatched, delete the cache
    let version_path = cache_dir.join("cas.version");
    let version_ok = if version_path.exists() {
        match fs::read_to_string(&version_path) {
            Ok(v) => v.trim().parse::<u32>().ok() == Some(CAS_VERSION),
            Err(_) => false,
        }
    } else {
        false
    };

    if !version_ok && blob_path.exists() {
        tracing::info!(
            "CAS version mismatch (expected v{}), deleting stale cache",
            CAS_VERSION
        );
        let _ = fs::remove_dir_all(&blob_path);
    }

    // Ensure blob directory exists
    fs::create_dir_all(&blob_path)?;

    // Write version file
    fs::write(&version_path, CAS_VERSION.to_string())?;

    let _ = BLOB_DIR.set(blob_path.clone());
    tracing::debug!("Blob storage initialized at {:?}", blob_path);
    Ok(())
}

/// Encode bytes as lowercase hex string
fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    result
}

/// Get the blob path for a given hash
fn blob_path(hash: &InputHash, extension: &str) -> Option<PathBuf> {
    let blob_dir = BLOB_DIR.get()?;
    // Use first 2 bytes as subdirectory for file system efficiency
    let hex = hex_encode(&hash.0);
    let subdir = &hex[0..4];
    Some(blob_dir.join(subdir).join(format!("{}.{}", hex, extension)))
}

/// Image processing pipeline version - bump this when encoding settings change
/// (widths, quality, formats, etc.) to invalidate the cache
pub const IMAGE_PIPELINE_VERSION: u64 = 1;

/// Hash of input image content (includes pipeline version)
#[derive(Clone, Copy, PartialEq, Eq, Hash, facet::Facet)]
pub struct InputHash(pub [u8; 32]);

/// Key for a specific image variant (format + size)
/// Used to compute deterministic cache-busted URLs without processing the image
#[derive(Hash)]
pub struct ImageVariantKey {
    /// Hash of input image content (includes pipeline version)
    pub input_hash: InputHash,
    /// Output format
    pub format: crate::image::OutputFormat,
    /// Output width in pixels
    pub width: u32,
}

impl ImageVariantKey {
    /// Compute a short hash suitable for cache-busting URLs
    /// Uses dodeca alphabet (base15) for a subtle signature
    pub fn url_hash(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = RapidHasher::default();
        self.hash(&mut hasher);
        crate::cache_bust::encode_dodeca(hasher.finish())
    }
}

/// Compute content hash for cache key (32 bytes for collision resistance)
/// Includes the pipeline version so changing settings invalidates cache
pub fn content_hash_32(data: &[u8]) -> InputHash {
    let mut result = [0u8; 32];

    // Hash with different seeds to get 32 bytes
    // Include pipeline version so changing settings invalidates cache
    let mut hasher = RapidHasher::default();
    hasher.write(&IMAGE_PIPELINE_VERSION.to_le_bytes());
    hasher.write(data);
    let h1 = hasher.finish();
    result[0..8].copy_from_slice(&h1.to_le_bytes());

    let mut hasher = RapidHasher::new(h1);
    hasher.write(data);
    let h2 = hasher.finish();
    result[8..16].copy_from_slice(&h2.to_le_bytes());

    let mut hasher = RapidHasher::new(h2);
    hasher.write(data);
    let h3 = hasher.finish();
    result[16..24].copy_from_slice(&h3.to_le_bytes());

    let mut hasher = RapidHasher::new(h3);
    hasher.write(data);
    let h4 = hasher.finish();
    result[24..32].copy_from_slice(&h4.to_le_bytes());

    InputHash(result)
}

/// Get cached processed images by input content hash
pub fn get_cached_image(content_hash: &InputHash) -> Option<ProcessedImages> {
    let path = blob_path(content_hash, "img")?;
    let data = fs::read(&path).ok()?;
    postcard::from_bytes(&data).ok()
}

/// Store processed images by input content hash
pub fn put_cached_image(content_hash: &InputHash, images: &ProcessedImages) {
    let Some(path) = blob_path(content_hash, "img") else {
        return;
    };
    let Ok(data) = postcard::to_allocvec(images) else {
        return;
    };

    // Ensure subdirectory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, &data);
}

// ============================================================================
// Font Decompression Cache
// ============================================================================

/// Font pipeline version - bump this when decompression/subsetting changes
pub const FONT_PIPELINE_VERSION: u64 = 1;

/// Get cached decompressed font (TTF) by input content hash
pub fn get_cached_decompressed_font(content_hash: &InputHash) -> Option<Vec<u8>> {
    let path = blob_path(content_hash, "ttf")?;
    fs::read(&path).ok()
}

/// Store decompressed font (TTF) by input content hash
pub fn put_cached_decompressed_font(content_hash: &InputHash, ttf_data: &[u8]) {
    let Some(path) = blob_path(content_hash, "ttf") else {
        return;
    };

    // Ensure subdirectory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, ttf_data);
}

/// Compute font content hash (includes font pipeline version)
pub fn font_content_hash(data: &[u8]) -> InputHash {
    let mut result = [0u8; 32];

    let mut hasher = RapidHasher::default();
    hasher.write(&FONT_PIPELINE_VERSION.to_le_bytes());
    hasher.write(data);
    let h1 = hasher.finish();
    result[0..8].copy_from_slice(&h1.to_le_bytes());

    let mut hasher = RapidHasher::new(h1);
    hasher.write(data);
    let h2 = hasher.finish();
    result[8..16].copy_from_slice(&h2.to_le_bytes());

    let mut hasher = RapidHasher::new(h2);
    hasher.write(data);
    let h3 = hasher.finish();
    result[16..24].copy_from_slice(&h3.to_le_bytes());

    let mut hasher = RapidHasher::new(h3);
    hasher.write(data);
    let h4 = hasher.finish();
    result[24..32].copy_from_slice(&h4.to_le_bytes());

    InputHash(result)
}
