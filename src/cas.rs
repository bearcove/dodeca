//! Content-addressed storage for incremental builds
//!
//! Uses rapidhash for fast hashing and canopydb for persistent storage.
//! Tracks which files have been written and their content hashes to avoid
//! unnecessary disk writes.
//!
//! Blobs (processed images, decompressed fonts) are stored on disk rather than
//! in the database to keep the database lean and fast for metadata queries.

use crate::db::ProcessedImages;
use camino::Utf8Path;
use canopydb::Database;
use rapidhash::fast::RapidHasher;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// Global asset cache instance (for metadata only)
static ASSET_CACHE: OnceLock<Database> = OnceLock::new();

// Global blob storage directory
static BLOB_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Content-addressed storage for build outputs
pub struct ContentStore {
    db: Database,
}

impl ContentStore {
    /// Open or create a content store at the given path
    pub fn open(path: &Utf8Path) -> color_eyre::Result<Self> {
        // canopydb stores data in a directory
        fs::create_dir_all(path)?;
        let db = Database::new(path.as_std_path())?;
        Ok(Self { db })
    }

    /// Compute the rapidhash of content
    fn hash(content: &[u8]) -> u64 {
        let mut hasher = RapidHasher::default();
        hasher.write(content);
        hasher.finish()
    }

    /// Write content to a file if it has changed since last build.
    /// Returns true if the file was written, false if skipped (unchanged).
    pub fn write_if_changed(&self, path: &Utf8Path, content: &[u8]) -> color_eyre::Result<bool> {
        let hash = Self::hash(content);
        let hash_bytes = hash.to_le_bytes();
        let path_key = path.as_str().as_bytes();

        // Check if we have a stored hash for this path
        let unchanged = {
            let rx = self.db.begin_read()?;
            if let Some(tree) = rx.get_tree(b"hashes")? {
                if let Some(stored) = tree.get(path_key)? {
                    stored.as_ref() == hash_bytes
                } else {
                    false
                }
            } else {
                false
            }
        };

        if unchanged {
            return Ok(false);
        }

        // Hash differs or not stored - write the file
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;

        // Update stored hash
        let tx = self.db.begin_write()?;
        let mut tree = tx.get_or_create_tree(b"hashes")?;
        tree.insert(path_key, &hash_bytes)?;
        drop(tree);
        tx.commit()?;

        Ok(true)
    }
}

// ============================================================================
// Asset Cache (global, for processed images, OG images, etc.)
// ============================================================================

/// Initialize the global asset cache
pub fn init_asset_cache(cache_dir: &Path) -> color_eyre::Result<()> {
    // canopydb stores data in a directory, not a single file
    let db_path = cache_dir.join("assets.canopy");

    // Blob storage directory (for processed images, decompressed fonts)
    let blob_path = cache_dir.join("blobs");

    // Ensure directories exist
    fs::create_dir_all(&db_path)?;
    fs::create_dir_all(&blob_path)?;

    let db = Database::new(&db_path)?;
    let _ = ASSET_CACHE.set(db);
    let _ = BLOB_DIR.set(blob_path.clone());
    tracing::info!("Asset cache initialized at {:?}", db_path);
    tracing::info!("Blob storage initialized at {:?}", blob_path);
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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
    let Some(path) = blob_path(content_hash, "img") else { return };
    let Ok(data) = postcard::to_allocvec(images) else { return };

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
    let Some(path) = blob_path(content_hash, "ttf") else { return };

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
