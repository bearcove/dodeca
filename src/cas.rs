//! Content-addressed storage for incremental builds
//!
//! Uses rapidhash for fast hashing and canopydb for persistent storage.
//! Tracks which files have been written and their content hashes to avoid
//! unnecessary disk writes.
//!
//! Also provides image caching to avoid re-processing images across restarts.

use crate::db::ProcessedImages;
use camino::Utf8Path;
use canopydb::Database;
use rapidhash::fast::RapidHasher;
use std::fs;
use std::hash::Hasher;
use std::path::Path;
use std::sync::OnceLock;

// Global image cache instance
static IMAGE_CACHE: OnceLock<Database> = OnceLock::new();

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
// Image Cache (global, for processed images)
// ============================================================================

/// Initialize the global image cache
pub fn init_image_cache(cache_dir: &Path) -> color_eyre::Result<()> {
    // canopydb stores data in a directory, not a single file
    let db_path = cache_dir.join("images.canopy");

    // Ensure the database directory exists
    fs::create_dir_all(&db_path)?;

    let db = Database::new(&db_path)?;
    let _ = IMAGE_CACHE.set(db);
    tracing::info!("Image cache initialized at {:?}", db_path);
    Ok(())
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
    pub fn url_hash(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = RapidHasher::default();
        self.hash(&mut hasher);
        let hash = hasher.finish();
        // Use first 8 chars of hex (32 bits) - enough for cache busting
        format!("{:08x}", hash as u32)
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
    let db = IMAGE_CACHE.get()?;
    let rx = db.begin_read().ok()?;
    let tree = rx.get_tree(b"processed").ok()??;
    let data = tree.get(&content_hash.0).ok()??;
    bincode::deserialize(&data).ok()
}

/// Store processed images by input content hash
pub fn put_cached_image(content_hash: &InputHash, images: &ProcessedImages) {
    let Some(db) = IMAGE_CACHE.get() else { return };
    let Ok(data) = bincode::serialize(images) else { return };

    let Ok(tx) = db.begin_write() else { return };
    let Ok(mut tree) = tx.get_or_create_tree(b"processed") else { return };
    let _ = tree.insert(&content_hash.0, &data);
    drop(tree);
    let _ = tx.commit();
}
